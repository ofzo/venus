use ratatui::prelude::*;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

/// Parse inline markdown formatting into ratatui Spans.
pub fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut current_text = String::new();

    while i < len {
        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, &['*', '*']) {
                if !current_text.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current_text)));
                }
                let inner: String = chars[i + 2..end].iter().collect();
                spans.push(Span::styled(
                    inner,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                i = end + 2;
                continue;
            }
        }

        // Italic: *text*
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(end) = find_single_closing(&chars, i + 1, '*') {
                if !current_text.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current_text)));
                }
                let inner: String = chars[i + 1..end].iter().collect();
                spans.push(Span::styled(
                    inner,
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
                i = end + 1;
                continue;
            }
        }

        // Inline code: `text`
        if chars[i] == '`' {
            if let Some(end) = find_single_closing(&chars, i + 1, '`') {
                if !current_text.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut current_text)));
                }
                let inner: String = chars[i + 1..end].iter().collect();
                spans.push(Span::styled(inner, Style::default().fg(Color::Cyan)));
                i = end + 1;
                continue;
            }
        }

        current_text.push(chars[i]);
        i += 1;
    }

    if !current_text.is_empty() {
        spans.push(Span::raw(current_text));
    }

    spans
}

/// Highlight a single code line using syntect, returning ratatui Spans.
pub fn highlight_code_line(line: &str, lang: &str) -> Vec<Span<'static>> {
    let syntax_set = SyntaxSet::load_defaults_newlines();
    let theme = ThemeSet::load_defaults()
        .themes
        .remove("base16-ocean.dark")
        .unwrap_or_else(|| {
            ThemeSet::load_defaults()
                .themes
                .into_values()
                .next()
                .unwrap()
        });

    let syntax = if !lang.is_empty() {
        syntax_set.find_syntax_by_token(lang)
    } else {
        None
    };

    if let Some(syntax) = syntax {
        let mut highlighter = HighlightLines::new(syntax, &theme);
        match highlighter.highlight_line(line, &syntax_set) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(style, text)| {
                    let fg = style.foreground;
                    Span::styled(
                        text.to_string(),
                        Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b)),
                    )
                })
                .collect(),
            Err(_) => vec![Span::raw(line.to_string())],
        }
    } else {
        vec![Span::raw(line.to_string())]
    }
}

fn find_closing(chars: &[char], start: usize, marker: &[char; 2]) -> Option<usize> {
    let mut j = start;
    while j + 1 < chars.len() {
        if chars[j] == marker[0] && chars[j + 1] == marker[1] {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_single_closing(chars: &[char], start: usize, marker: char) -> Option<usize> {
    for j in start..chars.len() {
        if chars[j] == marker {
            return Some(j);
        }
    }
    None
}
