use ratatui::{prelude::*, widgets::*};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, DisplayMessage, RenderSegment};
use crate::markdown_tui;
use crate::ui::{
    DIFF_ADD_BG, DIFF_ADD_FG, DIFF_ADD_SIGN, DIFF_HUNK_BG, DIFF_HUNK_FG, DIFF_LN_FG,
    DIFF_REMOVE_BG, DIFF_REMOVE_FG, DIFF_REMOVE_SIGN, SYS_META_FG, THEME_COLOR, USER_MSG_BG,
    USER_MSG_CAPTION_FG, USER_MSG_FG,
};

/// Render the scrollable message area.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    if app.messages.is_empty() {
        render_welcome(frame, area, app);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    // Cells available for the scrolling transcript; used to pad user
    // message cards out to the full row width so the light background
    // spans edge-to-edge regardless of text length / wrapping.
    let width: usize = area.width as usize;
    // Running 1-based ordinal over user messages (including slash-command
    // echoes). Reset naturally when the transcript is truncated, e.g. after
    // `/return #N`.
    let mut user_idx: usize = 0;

    for msg in &app.messages {
        match msg {
            DisplayMessage::User { text } => {
                user_idx = user_idx.saturating_add(1);
                // Render the whole user turn as a full-width, light-background
                // "card": each wrapped text line (bold white on the panel bg)
                // is right-padded with spaces so the bg spans the entire row,
                // and the `#N` ordinal is right-aligned on its own panel row
                // acting as a caption for the card. A trailing blank line then
                // separates the card from the next turn or assistant reply.
                let card_style = Style::default()
                    .fg(USER_MSG_FG)
                    .bg(USER_MSG_BG)
                    .add_modifier(Modifier::BOLD);
                let caption_style = Style::default().fg(USER_MSG_CAPTION_FG).bg(USER_MSG_BG);
                for seg in wrap_text_to_width(text, width) {
                    let mut spans: Vec<Span> = vec![Span::styled(seg.clone(), card_style)];
                    let seg_w = seg.width();
                    let pad_w = width.saturating_sub(seg_w);
                    if pad_w > 0 {
                        spans.push(Span::styled(" ".repeat(pad_w), card_style));
                    }
                    lines.push(Line::from(spans));
                }
                // `#N` caption, right-aligned within the same panel background.
                let num = format!("#{}", user_idx);
                let num_w = num.width();
                let pad_w = width.saturating_sub(num_w);
                lines.push(Line::from(vec![
                    Span::styled(" ".repeat(pad_w), caption_style),
                    Span::styled(num, caption_style),
                ]));
                lines.push(Line::from(""));
            }
            DisplayMessage::Assistant { segments } => {
                // Assistant text replies are rendered plainly: no glyph
                // prefix on the first (or any) line, so a plain answer
                // reads as just text and stays distinct from tool blocks.
                for RenderSegment::Text(text) in segments {
                    for line in render_markdown_with_code_blocks(text) {
                        lines.push(line);
                    }
                }
                lines.push(Line::from(""));
            }
            DisplayMessage::ToolCall {
                name,
                activity,
                is_error,
                output,
                diff,
            } => {
                // A tool use is the assistant's own action. The header is a
                // single bold coloured line carrying the tool name + its
                // invocation (NO glyph prefix on this first line, per the
                // design rule that `⏺` does not appear as a row prefix);
                // the tool's real (multi-line) output is drawn below it as a
                // box-drawing tree:
                //     Bash(git branch)
                //       │ <line>
                //       │ … +N lines        (collapsed middle)
                //       └ <last line>
                // header text + tree take red when `is_error`.
                let state_color = if *is_error { Color::Red } else { THEME_COLOR };
                let connector_color = Color::DarkGray;
                let content_color = if *is_error {
                    Color::Red
                } else {
                    Color::DarkGray
                };

                // Header: `Name(activity)` (no glyph prefix; no space before
                // the paren).
                let mut header: Vec<Span> = vec![
                    Span::styled(
                        name.clone(),
                        Style::default()
                            .fg(state_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                ];
                if !activity.is_empty() {
                    header.push(Span::styled(
                        format!("({})", activity),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(header));

                // ---------- Diff row styling ----------
                // Each diff row carries its own background band so the `+`/`-`
                // changes read as filled coloured strips (GitHub / delta
                // style), with an `old|new` line-number gutter on the left.
                // The whole row is right-padded to the viewport width so the
                // band spans edge-to-edge; context lines stay transparent so
                // they read like ordinary code, letting the changed strips pop.
                #[derive(Clone)]
                struct DiffRow {
                    conn: char,
                    old_ln: Option<usize>,
                    new_ln: Option<usize>,
                    sign: char,
                    text: String,
                    /// `None` (transparent) for context lines; a saturated band
                    /// for added / removed / hunk-header / collapse-marker rows.
                    bg: Option<Color>,
                    sign_fg: Color,
                    text_fg: Color,
                    ln_fg: Color,
                    conn_fg: Color,
                }

                fn push_diff_row(lines: &mut Vec<Line>, width: usize, row: &DiffRow) {
                    let old_str = match row.old_ln {
                        Some(n) => format!("{:>4}", n),
                        None => "    ".to_string(),
                    };
                    let new_str = match row.new_ln {
                        Some(n) => format!("{:>4}", n),
                        None => "    ".to_string(),
                    };
                    let conn_s = row.conn.to_string();
                    let sign_s = row.sign.to_string();
                    let pieces: Vec<(String, Color)> = vec![
                        ("  ".to_string(), row.conn_fg),
                        (conn_s, row.conn_fg),
                        (" ".to_string(), row.ln_fg),
                        (old_str, row.ln_fg),
                        (" ".to_string(), row.ln_fg),
                        (new_str, row.ln_fg),
                        (" ".to_string(), row.sign_fg),
                        (sign_s, row.sign_fg),
                        (" ".to_string(), row.sign_fg),
                        (row.text.clone(), row.text_fg),
                    ];
                    let used: usize = pieces.iter().map(|(t, _)| t.width()).sum();
                    let spans: Vec<Span> = pieces
                        .into_iter()
                        .map(|(t, fg)| {
                            let mut st = Style::default().fg(fg);
                            if let Some(b) = row.bg {
                                st = st.bg(b);
                            }
                            Span::styled(t, st)
                        })
                        .collect();
                    let mut spans = spans;
                    if let Some(b) = row.bg {
                        let pad = width.saturating_sub(used);
                        if pad > 0 {
                            spans.push(Span::styled(" ".repeat(pad), Style::default().bg(b)));
                        }
                    }
                    lines.push(Line::from(spans));
                }

                // ---------- Diff path (Write/Edit, non-error) ----------
                if let Some(tool_diff) = diff {
                    use venus_utils::diff::DiffLineKind;

                    // First body row: status line (the tool's plain output, e.g.
                    // "Wrote /path (42 lines)"), styled as a context row so the
                    // diff block reads as the body of the same tree.
                    let status_line = if output.trim().is_empty() {
                        tool_diff.path.clone()
                    } else {
                        output.clone()
                    };

                    const DIFF_HEAD: usize = 6;
                    const DIFF_MAX_FULL: usize = 12;

                    // Lightweight per-shown-row classification + metadata; the
                    // connector (`│`/`└`) and full visual styling are resolved
                    // in a second pass once we know total row count.
                    #[derive(Clone)]
                    enum Rk {
                        Hunk,
                        Ctx,
                        Add,
                        Rem,
                        Marker,
                    }
                    struct R {
                        rk: Rk,
                        old_ln: Option<usize>,
                        new_ln: Option<usize>,
                        text: String,
                    }

                    let mut rows: Vec<R> = Vec::new();
                    rows.push(R {
                        rk: Rk::Ctx,
                        old_ln: None,
                        new_ln: None,
                        text: status_line,
                    });

                    let dl = &tool_diff.lines;
                    let n = dl.len();
                    let to_row = |l: &venus_utils::diff::DiffLine| -> R {
                        let rk = match l.kind {
                            DiffLineKind::HunkHeader => Rk::Hunk,
                            DiffLineKind::Context => Rk::Ctx,
                            DiffLineKind::Add => Rk::Add,
                            DiffLineKind::Remove => Rk::Rem,
                        };
                        R {
                            rk,
                            old_ln: l.old_ln,
                            new_ln: l.new_ln,
                            text: l.text.clone(),
                        }
                    };
                    if n <= DIFF_MAX_FULL {
                        for l in dl.iter() {
                            rows.push(to_row(l));
                        }
                    } else {
                        // first DIFF_HEAD lines + collapse marker + last line
                        for l in dl.iter().take(DIFF_HEAD) {
                            rows.push(to_row(l));
                        }
                        let hidden = n - DIFF_HEAD - 1;
                        rows.push(R {
                            rk: Rk::Marker,
                            old_ln: None,
                            new_ln: None,
                            text: format!("\u{2026} +{} lines", hidden),
                        });
                        rows.push(to_row(&dl[n - 1]));
                    }

                    let total = rows.len();
                    for (i, r) in rows.iter().enumerate() {
                        let conn = if i + 1 == total { '\u{2514}' } else { '\u{2502}' };
                        let row = match r.rk {
                            Rk::Add => DiffRow {
                                conn,
                                old_ln: r.old_ln,
                                new_ln: r.new_ln,
                                sign: '+',
                                text: r.text.clone(),
                                bg: Some(DIFF_ADD_BG),
                                sign_fg: DIFF_ADD_SIGN,
                                text_fg: DIFF_ADD_FG,
                                ln_fg: DIFF_LN_FG,
                                conn_fg: connector_color,
                            },
                            Rk::Rem => DiffRow {
                                conn,
                                old_ln: r.old_ln,
                                new_ln: r.new_ln,
                                sign: '-',
                                text: r.text.clone(),
                                bg: Some(DIFF_REMOVE_BG),
                                sign_fg: DIFF_REMOVE_SIGN,
                                text_fg: DIFF_REMOVE_FG,
                                ln_fg: DIFF_LN_FG,
                                conn_fg: connector_color,
                            },
                            Rk::Hunk => DiffRow {
                                conn,
                                old_ln: r.old_ln,
                                new_ln: r.new_ln,
                                sign: ' ',
                                text: r.text.clone(),
                                bg: Some(DIFF_HUNK_BG),
                                sign_fg: DIFF_HUNK_FG,
                                text_fg: DIFF_HUNK_FG,
                                ln_fg: DIFF_HUNK_FG,
                                conn_fg: connector_color,
                            },
                            Rk::Marker => DiffRow {
                                conn,
                                old_ln: r.old_ln,
                                new_ln: r.new_ln,
                                sign: ' ',
                                text: r.text.clone(),
                                bg: Some(DIFF_HUNK_BG),
                                sign_fg: DIFF_HUNK_FG,
                                text_fg: DIFF_HUNK_FG,
                                ln_fg: DIFF_HUNK_FG,
                                conn_fg: connector_color,
                            },
                            // Context: transparent band so it reads like code.
                            Rk::Ctx => DiffRow {
                                conn,
                                old_ln: r.old_ln,
                                new_ln: r.new_ln,
                                sign: ' ',
                                text: r.text.clone(),
                                bg: None,
                                sign_fg: content_color,
                                text_fg: content_color,
                                ln_fg: Color::Reset,
                                conn_fg: connector_color,
                            },
                        };
                        push_diff_row(&mut lines, width, &row);
                    }
                } else {
                    // ---------- Plain tree path (non-diff tools) ----------
                    let body: Vec<&str> = output.lines().collect();
                    const HEAD: usize = 2;
                    const MAX_FULL: usize = 5;
                    if !body.is_empty() {
                        let n = body.len();
                        let rows: Vec<(char, String, Color)> = if n <= MAX_FULL {
                            body.iter()
                                .enumerate()
                                .map(|(i, l)| {
                                    let conn = if i + 1 == n { '\u{2514}' } else { '\u{2502}' };
                                    (conn, l.to_string(), content_color)
                                })
                                .collect()
                        } else {
                            let hidden = n - HEAD - 1;
                            let mut rows: Vec<(char, String, Color)> = Vec::new();
                            for l in body.iter().take(HEAD) {
                                rows.push(('\u{2502}', l.to_string(), content_color));
                            }
                            rows.push((
                                '\u{2502}',
                                format!("\u{2026} +{} lines", hidden),
                                connector_color,
                            ));
                            rows.push(('\u{2514}', body[n - 1].to_string(), content_color));
                            rows
                        };
                        for (conn, text, col) in rows {
                            lines.push(Line::from(vec![
                                Span::styled("  ", Style::default()),
                                Span::styled(conn.to_string(), Style::default().fg(connector_color)),
                                Span::styled(" ", Style::default()),
                                Span::styled(text, Style::default().fg(col)),
                            ]));
                        }
                    }
                }
                lines.push(Line::from(""));
            }
            DisplayMessage::Error { text } => {
                // System-error line: red ✕ glyph (the glyph carries the
                // "error" intent, so the legacy "Error:" word is dropped) + red
                // text. Tool errors and system errors share the red ✕ so
                // "failure" reads consistently across categories.
                lines.push(Line::from(vec![
                    Span::styled(
                        "  \u{2715} ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(text.clone(), Style::default().fg(Color::Red)),
                ]));
            }
            DisplayMessage::Status { text } => {
                // System meta rendered as a full-width dashed divider so it
                // reads as UI "chrome" rather than a conversation turn:
                //   `------ text ------` spanning the whole row in muted
                //   gray, text centred between equal-ish dash groups. Falls
                //   back to the bare text when it alone nearly fills the row.
                let style = Style::default().fg(SYS_META_FG);
                let text_w = text.as_str().width();
                let mut built: String = String::with_capacity(width);
                if text_w + 2 >= width {
                    built.push_str(text);
                } else {
                    let sides = width - text_w - 2; // two flanking spaces
                    let left = sides / 2;
                    built.push_str(&"-".repeat(left));
                    built.push(' ');
                    built.push_str(text);
                    built.push(' ');
                    built.push_str(&"-".repeat(sides - left));
                }
                lines.push(Line::from(vec![Span::styled(built, style)]));
            }
        }
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

    let scroll = if app.auto_scroll {
        (0, 0)
    } else {
        (app.scroll_offset, 0)
    };

    let paragraph = paragraph.scroll(scroll);

    frame.render_widget(paragraph, area);
}

/// Render the welcome banner shown when the conversation is empty.
///
/// Mirrors the Codex/Claude-Code startup header: a rounded box that shows the
/// Venus version, active model + effort, and the working directory, followed by
/// a short tip line. Once any message is present this is replaced by the normal
/// scrolling transcript.
fn render_welcome(frame: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let model = app.model();
    let effort = app.get_effort_label();
    let dir = shorten_home(&app.engine.working_dir);
    render_welcome_inner(frame, area, version, model, effort, &dir);
}

fn render_welcome_inner(
    frame: &mut Frame,
    area: Rect,
    version: &str,
    model: &str,
    effort: &str,
    dir: &str,
) {
    frame.render_widget(Clear, area);

    // Accent column prefix shared by every banner line; a single leading
    // space keeps the accent column off the screen's left edge.
    let prefix = Span::styled(" \u{2588} ", Style::default().fg(THEME_COLOR)); // █

    let title = format!(">_< Venus (v{})", version);
    let model_line = vec![
        prefix.clone(),
        Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
        Span::raw(model.to_string()),
        Span::raw(" "),
        Span::raw(effort.to_string()),
        Span::styled("  /model to change", Style::default().fg(Color::DarkGray)),
    ];
    let dir_line = vec![
        prefix.clone(),
        Span::styled("Directory: ", Style::default().fg(Color::DarkGray)),
        Span::raw(dir.to_string()),
    ];

    let lines = vec![
        Line::from(vec![prefix.clone(), Span::raw(title)]),
        Line::from(prefix.clone()), // blank accent line
        Line::from(model_line),
        Line::from(dir_line),
    ];

    // No border box: just the accent-prefixed lines, dropped one row below
    // the top of the messages area so the banner has a small upper margin.
    let height = lines.len() as u16;
    let y = area.y.saturating_add(1);
    if y + height > area.bottom() {
        return;
    }
    let banner_area = Rect::new(area.x, y, area.width, height);
    frame.render_widget(Paragraph::new(lines), banner_area);
}

/// Render an absolute path with the home directory shortened to `~`,
/// e.g. `/Users/wei/code/playground/venus` -> `~/code/playground/venus`.
pub(crate) fn shorten_home(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Greedy character-based word wrap to a fixed display width.
///
/// Splits on existing newlines first (preserving explicit paragraph breaks),
/// then breaks each paragraph greedily by Unicode display width so wide
/// (e.g. CJK) glyphs are accounted for. Wrapping is character-level rather
/// than word-level, which is fine for user prompts that are typically short;
/// it also guarantees every returned segment is at most `width` cells wide so
/// the caller can right-pad each segment to the full panel width without
/// triggering ratatui's own wrap re-flow.
fn wrap_text_to_width(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if width == 0 || paragraph.is_empty() {
            out.push(paragraph.to_string());
            continue;
        }
        let mut line = String::new();
        let mut line_w: usize = 0;
        for ch in paragraph.chars() {
            let cw = ch.width().unwrap_or(0);
            if line_w + cw > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                line_w = 0;
            }
            line.push(ch);
            line_w += cw;
        }
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Render markdown text with code block support.
fn render_markdown_with_code_blocks(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in text.split('\n') {
        // Check for code block start/end
        if let Some(stripped) = line.strip_prefix("```") {
            if in_code_block {
                // End of code block
                in_code_block = false;
                lines.push(Line::from(Span::styled(
                    "  ────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                // Start of code block
                in_code_block = true;
                code_lang = stripped.trim().to_string();
                lines.push(Line::from(Span::styled(
                    format!(
                        "  ┌─ {} ─────────────────────────────",
                        if code_lang.is_empty() {
                            "code"
                        } else {
                            &code_lang
                        }
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            continue;
        }

        if in_code_block {
            // Render code line with syntax highlighting
            let highlighted = markdown_tui::highlight_code_line(line, &code_lang);
            let mut spans = vec![Span::styled("  │ ", Style::default().fg(Color::DarkGray))];
            spans.extend(highlighted);
            lines.push(Line::from(spans));
        } else {
            // Regular markdown line
            lines.push(render_markdown_line(line));
        }
    }

    lines
}

/// Render a single markdown line with inline formatting.
fn render_markdown_line(line: &str) -> Line<'static> {
    // Headers
    if let Some(content) = line.strip_prefix("#### ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("### ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("## ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(content) = line.strip_prefix("# ") {
        return Line::from(Span::styled(
            content.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Horizontal rule
    let trimmed = line.trim();
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Unordered list
    if let Some(content) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("  • ", Style::default().fg(THEME_COLOR))];
        spans.extend(markdown_tui::parse_inline(content));
        return Line::from(spans);
    }

    // Indented list
    if let Some(rest) = line
        .strip_prefix("  - ")
        .or_else(|| line.strip_prefix("  * "))
    {
        let mut spans = vec![Span::styled("    ◦ ", Style::default().fg(THEME_COLOR))];
        spans.extend(markdown_tui::parse_inline(rest));
        return Line::from(spans);
    }

    // Ordered list
    if let Some(dot_pos) = line.find(". ") {
        if dot_pos <= 3 && line[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
            let num = &line[..dot_pos];
            let content = &line[dot_pos + 2..];
            let mut spans = vec![Span::styled(
                format!("  {}. ", num),
                Style::default().fg(THEME_COLOR),
            )];
            spans.extend(markdown_tui::parse_inline(content));
            return Line::from(spans);
        }
    }

    // Blockquote
    if let Some(content) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                content.to_string(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
    }

    // Regular text with inline formatting
    let spans = markdown_tui::parse_inline(line);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_to_string(
        width: u16,
        height: u16,
        version: &str,
        model: &str,
        effort: &str,
        dir: &str,
    ) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, width, height);
                render_welcome_inner(f, area, version, model, effort, dir);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                let sym = buf[(x, y)].symbol();
                out.push_str(sym);
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn welcome_renders_accent_banner() {
        let rendered = render_to_string(
            80,
            10,
            "0.1.0",
            "claude-sonnet-4-20250514",
            "medium",
            "~/code/playground/venus",
        );

        // No rounded-box border anymore; the banner is borderless `\u{2588}`-prefixed lines.
        assert!(!rendered.contains('\u{256D}'), "no top-left box corner");
        assert!(!rendered.contains('\u{256E}'), "no top-right box corner");
        assert!(!rendered.contains('\u{2570}'), "no bottom-left box corner");
        assert!(!rendered.contains('\u{256F}'), "no bottom-right box corner");
        assert!(!rendered.contains('\u{2502}'), "no box side border");

        // Accent column prefix is present.
        assert!(rendered.contains('\u{2588}'), "accent block char present");

        // Header content.
        assert!(rendered.contains(">_< Venus (v0.1.0)"));
        assert!(rendered.contains("Model: claude-sonnet-4-20250514 medium"));
        assert!(rendered.contains("/model to change"));
        assert!(rendered.contains("Directory: ~/code/playground/venus"));

        // At least one visible line must be flush-left (starts with the accent char).
        assert!(
            rendered.lines().any(|l| l.starts_with(" \u{2588}")),
            "banner has a 1-space left margin then the accent column"
        );

        // Old tip line was removed (it moved to the bottom status bar).
        assert!(!rendered.contains("Tip: Use /init"));
    }

    #[test]
    fn welcome_is_left_aligned_in_narrow_area() {
        // Narrow terminal: should not panic and should still fit the accent title.
        let rendered = render_to_string(
            30,
            10,
            "0.1.0",
            "claude-sonnet-4-20250514",
            "medium",
            "~/code/playground/venus",
        );
        assert!(rendered.contains("Venus (v0.1.0)"));
        assert!(!rendered.contains('\u{256D}'));
        assert!(!rendered.contains('\u{256F}'));
    }
}

#[test]
fn wrap_splits_long_line_at_display_width() {
    // ASCII: width 5 -> each row holds 5 chars.
    let out = wrap_text_to_width("abcdefgh", 5);
    assert_eq!(out, vec!["abcde".to_string(), "fgh".to_string()]);
}

#[test]
fn wrap_respects_newlines_as_paragraph_breaks() {
    let out = wrap_text_to_width("ab\ncd", 10);
    assert_eq!(out, vec!["ab".to_string(), "cd".to_string()]);
}

#[test]
fn wrap_empty_returns_single_empty_line() {
    let out = wrap_text_to_width("", 10);
    assert_eq!(out, vec!["".to_string()]);
}

#[test]
fn wrap_zero_width_falls_back_to_single_line() {
    let out = wrap_text_to_width("abc", 0);
    assert_eq!(out, vec!["abc".to_string()]);
}

#[test]
fn wrap_accounts_for_wide_cjk_glyphs() {
    // Two full-width glyphs = 4 cells fit in width 5; a 3rd would make 6 > 5,
    // so greedy wrap flushes after each pair, leaving the lone last glyph.
    let out = wrap_text_to_width("一二三四五", 5);
    assert_eq!(
        out,
        vec!["一二".to_string(), "三四".to_string(), "五".to_string()]
    );
}
