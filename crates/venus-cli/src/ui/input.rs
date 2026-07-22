use ratatui::{prelude::*, widgets::*};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, InputMode};
use crate::ui::THEME_COLOR;

/// Render the input area matching Claude Code's PromptInput exactly.
///
/// Claude Code layout:
///   Box marginTop={1}
///   Box borderStyle="round" borderLeft=false borderRight=false borderBottom
///     ❯ user input text
///   PromptInputFooter
///
/// Key: ONLY bottom border is shown (not top, not left, not right).
/// Accent colours for the two pill token classes. File references use the
/// cyan theme family; invoked skills use magenta, so the two stay visually
/// distinct inside the prompt line.
const FILE_FG: Color = Color::Rgb(0xB3, 0x00, 0x1E); // deep rose on light rose
const FILE_BG: Color = Color::Rgb(0xF5, 0xD7, 0xDC);
const SKILL_FG: Color = Color::Rgb(0xB1, 0x61, 0xBF); // lilac (oklch 62% 0.16 321.4)
const SKILL_BG: Color = Color::Rgb(0xF6, 0xDF, 0xFA);

/// Solid Powerline rounded-cap glyphs for a seamless pill chip. The pair is
/// the *solid* twins, not the outline ones: U+E0B4/U+E0B6 are filled, their
/// hollow twins U+E0B5/U+E0B7 would render as thin lines. Verified solid in
/// both Cascadia Mono NF and JetBrainsMono NF (ink density ~0.8 vs ~0.2 for
/// the outlines). U+E0B6 is the left cap (solid ink faces the content on the
/// right); U+E0B4 is the mirror right cap (solid ink faces content on the
/// left). Both caps are painted fg=segment-bg / bg=terminal-default. Nerd Font
/// (PUA); falls back to another NF face if the primary one lacks the glyph.
const PILL_LEFT: &str = "\u{E0B6}"; //  solid rounded left cap (ink->right)
const PILL_RIGHT: &str = "\u{E0B4}"; //  solid rounded right cap (ink->left)

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    // 2-space left pad so the typed text (and the placeholder) is not flush
    // against the screen's left edge.
    let pad = Span::raw("  ");

    let (input_spans, extra_cells) =
        if app.input.buffer.is_empty() && app.input_mode == InputMode::Normal {
            (
                vec![
                    pad,
                    Span::styled(
                        "Type a message\u{2026}",
                        Style::default().fg(Color::DarkGray),
                    ),
                ],
                0u16,
            )
        } else {
            build_input_spans(app, pad)
        };
    let input_text = Line::from(input_spans);

    // Claude Code: borderStyle="round", borderLeft=false, borderRight=false,
    // borderTop=true, borderBottom=true  -> lines above and below the prompt only
    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM) // lines above & below the prompt
        .border_style(Style::default().fg(THEME_COLOR))
        .border_type(BorderType::Rounded);

    let paragraph = Paragraph::new(input_text).block(block);
    frame.render_widget(paragraph, area);

    // Show cursor in normal mode only. The caret x must account for any pill
    // delimiters (`◖`/`◗`, one cell each) injected before the cursor so it
    // stays aligned with the glyph being edited.
    if app.input_mode == InputMode::Normal {
        let cp = app.input.cursor_pos.min(app.input.buffer.len());
        let to_cursor_w = UnicodeWidthStr::width(&app.input.buffer[..cp]) as u16;
        let cursor_x = area
            .x
            .saturating_add(2)
            .saturating_add(to_cursor_w)
            .saturating_add(extra_cells)
            .min(area.x.saturating_add(area.width).saturating_sub(1));
        // Content row is the single line between the top and bottom borders
        // (input area is Height(3): border, content, border).
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));
    }

    // Show completion popup if active (Claude-Code style, above the input box).
    if !app.input.completion_items.is_empty() {
        render_completion_popup(frame, area, app);
    }
}

/// Build the prompt line spans, wrapping finished `@file` references and
/// `/skill-name` invocations in coloured pills (`◖ … ◗`). The token under the
/// caret is left as plain editable text so it composes naturally with inline
/// ghost preview and the completion popup. Returns the spans plus the number
/// of injected pill-delimiter cells that precede the caret (for caret x).
fn build_input_spans(app: &App, pad: Span<'static>) -> (Vec<Span<'static>>, u16) {
    let buf = &app.input.buffer;
    let cp = app.input.cursor_pos.min(buf.len());
    let toks = tokenize_input(buf);

    let mut spans: Vec<Span> = Vec::with_capacity(toks.len() * 3 + 4);
    spans.push(pad);

    let mut extra: u16 = 0;
    let mut last_end: usize = 0;
    for tok in &toks {
        if tok.start > last_end {
            spans.push(Span::raw(buf[last_end..tok.start].to_string()));
        }
        let active = cp >= tok.start && cp <= tok.end;
        let before_caret = tok.end <= cp;
        if active {
            // Plain editable text; ghost preview is appended after the loop.
            spans.push(Span::raw(buf[tok.start..tok.end].to_string()));
        } else {
            match tok.kind {
                Tk::Text => spans.push(Span::raw(buf[tok.start..tok.end].to_string())),
                Tk::File => {
                    if before_caret {
                        extra = extra.saturating_add(2);
                    }
                    push_pill(&mut spans, &buf[tok.start..tok.end], FILE_FG, FILE_BG);
                }
                Tk::Slash => {
                    // `/name` is a skill pill only when it resolves to a
                    // user-invocable skill (mirrors slash-command dispatch).
                    let name = &buf[tok.start + 1..tok.end];
                    let is_skill = app
                        .skill_registry
                        .as_ref()
                        .and_then(|r| r.find(name))
                        .map(|s| s.user_invocable)
                        .unwrap_or(false);
                    if is_skill {
                        if before_caret {
                            extra = extra.saturating_add(2);
                        }
                        push_pill(&mut spans, &buf[tok.start..tok.end], SKILL_FG, SKILL_BG);
                    } else {
                        spans.push(Span::raw(buf[tok.start..tok.end].to_string()));
                    }
                }
            }
        }
        last_end = tok.end;
    }
    if last_end < buf.len() {
        spans.push(Span::raw(buf[last_end..].to_string()));
    }
    if let Some(g) = &app.input.ghost_text {
        spans.push(Span::styled(
            g.clone(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    (spans, extra)
}

/// Append a seamless pill chip using the solid Powerline rounded-cap pair
/// (U+E0B6 left, U+E0B4 right -- NOT U+E0B7, which is the outline twin and
/// renders as a thin line). Both caps are painted fg=segment-bg /
/// bg=terminal-default so each cap's chip-coloured solid ink meets the
/// neighbouring content cell and the rounded outer edge reads against the
/// terminal default behind the prompt (symmetric starship/oh-my-posh rounded
/// convention, valid because the two solid glyphs are true mirrors). Requires
/// a Nerd Font (PUA).
fn push_pill(spans: &mut Vec<Span>, content: &str, fg: Color, bg: Color) {
    let cap = Style::default().fg(bg).bg(Color::Reset);
    spans.push(Span::styled(PILL_LEFT, cap));
    spans.push(Span::styled(
        content.to_string(),
        Style::default().fg(fg).bg(bg),
    ));
    spans.push(Span::styled(PILL_RIGHT, cap));
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tk {
    /// Plain text (incl. whitespace separators).
    Text,
    /// A file reference beginning with `@`.
    File,
    /// A slash token beginning with `/` at a word boundary.
    Slash,
}

struct TokenSpan {
    kind: Tk,
    start: usize,
    end: usize,
}

/// Split the prompt buffer into tokens for pill rendering. `@` starts a file
/// token; `/` starts a slash token only when it sits at a word boundary
/// (buffer start or after whitespace), so path separators inside `@path`
/// or `a/b` stay inside their text run. Bytes are scanned directly: `@`, `/`
/// and ASCII whitespace never occur inside a UTF-8 multibyte sequence, so
/// byte-level scans cannot split a multibyte character.
fn tokenize_input(buf: &str) -> Vec<TokenSpan> {
    fn is_ws(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | b'\n' | b'\r')
    }
    let b = buf.as_bytes();
    let n = b.len();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < n {
        let cur = b[i];
        let prev_ws = i == 0 || is_ws(b[i - 1]);
        if cur == b'@' {
            let start = i;
            i += 1;
            while i < n && !is_ws(b[i]) {
                i += 1;
            }
            toks.push(TokenSpan {
                kind: Tk::File,
                start,
                end: i,
            });
        } else if cur == b'/' && prev_ws {
            let start = i;
            i += 1;
            while i < n && !is_ws(b[i]) {
                i += 1;
            }
            toks.push(TokenSpan {
                kind: Tk::Slash,
                start,
                end: i,
            });
        } else {
            let start = i;
            i += 1;
            while i < n {
                let c = b[i];
                if c == b'@' || (c == b'/' && is_ws(b[i - 1])) {
                    break;
                }
                i += 1;
            }
            toks.push(TokenSpan {
                kind: Tk::Text,
                start,
                end: i,
            });
        }
    }
    toks
}

/// Draw the Claude-Code style completion popup directly above the input box.
///
/// - Full width of the input area, up to 8 rows.
/// - Table layout: a left-aligned label column (sized to the widest label in
///   the list) followed by a fixed gap, then a left-aligned description column.
///   All descriptions share the same start column, so short descriptions line
///   up instead of hugging the right edge.
/// - Selected row: light theme-colour background fill across the full width.
/// - Matched query chars: theme colour foreground.
/// - Other chars and description: gray.
/// - Whole panel: light gray background.
fn render_completion_popup(frame: &mut Frame, input_area: Rect, app: &App) {
    let items = &app.input.completion_items;
    if items.is_empty() {
        return;
    }
    let rows = (items.len() as u16).min(8);
    let y = input_area.y.saturating_sub(rows);
    let area = Rect::new(input_area.x, y, input_area.width, rows);

    // Palette (tunable).
    let panel_bg: Color = Color::Rgb(0xEC, 0xEF, 0xF3); // light gray
    let selected_bg: Color = Color::Rgb(0xF5, 0xD7, 0xDC); // light theme (rose) fill
    let match_fg: Color = THEME_COLOR; // matched chars: theme colour
    let other_fg: Color = Color::DarkGray; // other chars / description: gray

    // Erase the messages text behind the popup so it reads as a panel.
    frame.render_widget(Clear, area);

    // Width of the label column = widest label across all visible rows, using
    // real display width so CJK/emoji labels don't desync the second column.
    let label_col = items
        .iter()
        .take(8)
        .map(|it| it.label.width() as u16)
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line> = Vec::new();
    for (i, item) in items.iter().take(8).enumerate() {
        let selected = i == app.input.completion_index;
        let bg = if selected { selected_bg } else { panel_bg };

        let label_w = item.label.width() as u16;
        let desc_w = item.description.width() as u16;
        let label_fill = label_col.saturating_sub(label_w);
        let trailing = area.width.saturating_sub(2u16 + label_col + 2u16 + desc_w);

        let mut spans: Vec<Span> = Vec::with_capacity(7);
        // 2-space left pad so text is not flush against the panel edge.
        spans.push(Span::styled("  ", Style::default().bg(bg)));
        // Build per-char label spans: matched chars use the theme colour,
        // the rest use gray. Adjacent chars with the same style merge into
        // one Span so the label column stays a single visual run per style.
        let mut buf = String::new();
        let mut cur_match = false;
        let matched_len = item.matched.len();
        for (ci, ch) in item.label.chars().enumerate() {
            let is_match = ci < matched_len && item.matched[ci];
            if buf.is_empty() {
                cur_match = is_match;
                buf.push(ch);
                continue;
            }
            if is_match == cur_match {
                buf.push(ch);
            } else {
                let fg = if cur_match { match_fg } else { other_fg };
                spans.push(Span::styled(buf.clone(), Style::default().fg(fg).bg(bg)));
                buf.clear();
                cur_match = is_match;
                buf.push(ch);
            }
        }
        if !buf.is_empty() {
            let fg = if cur_match { match_fg } else { other_fg };
            spans.push(Span::styled(buf, Style::default().fg(fg).bg(bg)));
        }
        // Pad the label column to a uniform width so descriptions start aligned.
        if label_fill > 0 {
            spans.push(Span::styled(
                " ".repeat(label_fill as usize),
                Style::default().bg(bg),
            ));
        }
        // Fixed gap between the two columns.
        spans.push(Span::styled("  ", Style::default().bg(bg)));
        if !item.description.is_empty() {
            spans.push(Span::styled(
                item.description.clone(),
                Style::default().fg(other_fg).bg(bg),
            ));
        }
        // Fill any trailing cells so the row background spans the full width.
        if trailing > 0 {
            spans.push(Span::styled(
                " ".repeat(trailing as usize),
                Style::default().bg(bg),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Cover any short trailing cells with the panel background.
    let panel_fill = Block::default().style(Style::default().bg(panel_bg));
    frame.render_widget(panel_fill, area);
    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(buf: &str) -> Vec<(Tk, usize, usize)> {
        tokenize_input(buf)
            .into_iter()
            .map(|t| (t.kind, t.start, t.end))
            .collect()
    }

    #[test]
    fn tokenize_file_ref_and_slash_command() {
        // `@src/lib.rs` is one file token; `some text` is plain; the `/clear`
        // (builtin, but tokenized as Slash for the renderer to classify) sits
        // at a word boundary.
        let toks = kinds("@src/lib.rs fix this /clear");
        assert_eq!(
            toks,
            vec![
                (Tk::File, 0, 11),   // @src/lib.rs
                (Tk::Text, 11, 21),  // " fix this "
                (Tk::Slash, 21, 27), // /clear
            ]
        );
    }

    #[test]
    fn slash_inside_path_is_not_a_command() {
        // A `/` not at a word boundary stays inside the text/text-run, so
        // paths like a/b/c are not misparsed as commands.
        let toks = kinds("see a/b/c");
        assert!(toks.iter().all(|(k, _, _)| *k != Tk::Slash));
        // Only one text token here (no leading @).
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].0, Tk::Text);
    }

    #[test]
    fn slash_at_start_is_a_command() {
        let toks = kinds("/skills");
        assert_eq!(toks, vec![(Tk::Slash, 0, 7)]);
    }

    #[test]
    fn multiple_file_refs() {
        let toks = kinds("@a @b");
        // @a (0..2), " " (2..3), @b (3..5)
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[0], (Tk::File, 0, 2));
        assert_eq!(toks[1], (Tk::Text, 2, 3));
        assert_eq!(toks[2], (Tk::File, 3, 5));
    }

    #[test]
    fn multibyte_path_keeps_char_boundaries() {
        // A CJK filename must not be split: the `@` token absorbs the whole
        // multibyte run without breaking on continuation bytes.
        let toks = kinds("@中文.rs done");
        assert_eq!(toks[0], (Tk::File, 0, "@中文.rs".len()));
        assert_eq!(toks[1].0, Tk::Text);
    }
}
