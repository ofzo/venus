use std::io::{self, Write};
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use unicode_width::UnicodeWidthChar;

/// Get the current terminal width, defaulting to 80 if unavailable.
fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
}

/// Calculate the display width of a string, ignoring ANSI escape sequences
/// and accounting for CJK double-width characters.
#[allow(dead_code)]
fn display_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            in_escape = true;
            continue;
        }
        width += ch.width().unwrap_or(0);
    }
    width
}

/// Wrap a line of text (which may contain ANSI escapes) to fit within `max_width`.
/// Continuation lines are indented by `indent` spaces.
/// Uses \r\n for line breaks (required when terminal is in raw mode).
fn wrap_line(text: &str, max_width: usize, indent: usize) -> String {
    if max_width <= indent + 4 {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len() + 32);
    let mut current_width = 0;
    let mut first_line = true;
    let mut in_escape = false;
    let mut escape_buf = String::new();
    let mut active_escapes: Vec<String> = Vec::new();

    for ch in text.chars() {
        if in_escape {
            escape_buf.push(ch);
            if ch.is_ascii_alphabetic() {
                in_escape = false;
                let esc = std::mem::take(&mut escape_buf);
                if esc == "[0m" {
                    active_escapes.clear();
                } else {
                    active_escapes.push(esc.clone());
                }
                result.push('\x1b');
                result.push_str(&esc);
            }
            continue;
        }
        if ch == '\x1b' {
            in_escape = true;
            escape_buf.clear();
            continue;
        }

        let ch_width = ch.width().unwrap_or(0);
        let available = if first_line { max_width } else { max_width - indent };

        if current_width + ch_width > available && current_width > 0 {
            if !active_escapes.is_empty() {
                result.push_str("\x1b[0m");
            }
            result.push_str("\r\n");
            first_line = false;
            for _ in 0..indent {
                result.push(' ');
            }
            for esc in &active_escapes {
                result.push('\x1b');
                result.push_str(esc);
            }
            current_width = 0;
        }

        result.push(ch);
        current_width += ch_width;
    }

    result
}

/// Newline sequence for raw-mode terminal output.
const NL: &str = "\r\n";

/// A streaming markdown renderer for terminal output.
///
/// Handles both thinking text and response text with proper wrapping.
/// Uses \r\n for line breaks since reedline keeps the terminal in raw mode
/// during response rendering.
pub struct MarkdownRenderer {
    line_buf: String,
    in_code_block: bool,
    code_lang: String,
    code_buf: String,
    syntax_set: SyntaxSet,
    theme: Theme,
    term_width: usize,
    thinking_buf: String,
    was_thinking: bool,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme = ThemeSet::load_defaults()
            .themes
            .remove("base16-ocean.dark")
            .unwrap_or_else(|| ThemeSet::load_defaults().themes.into_values().next().unwrap());

        Self {
            line_buf: String::new(),
            in_code_block: false,
            code_lang: String::new(),
            code_buf: String::new(),
            syntax_set,
            theme,
            term_width: terminal_width(),
            thinking_buf: String::new(),
            was_thinking: false,
        }
    }

    /// Feed a thinking text delta. Buffered by line and output wrapped + dimmed.
    pub fn push_thinking(&mut self, text: &str) {
        self.was_thinking = true;
        let stderr = io::stderr();
        let mut out = stderr.lock();

        for ch in text.chars() {
            if ch == '\n' {
                let line = std::mem::take(&mut self.thinking_buf);
                let wrapped = wrap_line(&line, self.term_width, 0);
                let _ = write!(out, "\x1b[2m{}\x1b[0m{}", wrapped, NL);
            } else {
                self.thinking_buf.push(ch);
            }
        }
    }

    fn finish_thinking(&mut self) {
        if !self.thinking_buf.is_empty() {
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let line = std::mem::take(&mut self.thinking_buf);
            let wrapped = wrap_line(&line, self.term_width, 0);
            let _ = write!(out, "\x1b[2m{}\x1b[0m{}", wrapped, NL);
        }
        if self.was_thinking {
            self.was_thinking = false;
            let stderr = io::stderr();
            let mut out = stderr.lock();
            let _ = write!(out, "{}", NL);
        }
    }

    /// Feed a text delta from the streaming API.
    pub fn push(&mut self, text: &str) {
        self.finish_thinking();

        for ch in text.chars() {
            if ch == '\n' {
                self.flush_line();
            } else {
                self.line_buf.push(ch);
            }
        }
    }

    /// Flush any remaining buffered content (call on MessageComplete).
    pub fn finish(&mut self) {
        self.finish_thinking();
        if !self.line_buf.is_empty() {
            self.flush_line();
        }
        if self.in_code_block {
            self.render_code_block();
        }
    }

    fn flush_line(&mut self) {
        let line = std::mem::take(&mut self.line_buf);

        if self.in_code_block {
            if line.starts_with("```") {
                self.render_code_block();
            } else {
                self.code_buf.push_str(&line);
                self.code_buf.push('\n');
            }
            return;
        }

        if line.starts_with("```") {
            self.in_code_block = true;
            self.code_lang = line[3..].trim().to_string();
            self.code_buf.clear();
            return;
        }

        self.render_line(&line);
    }

    fn render_line(&self, line: &str) {
        let stderr = io::stderr();
        let mut out = stderr.lock();
        let w = self.term_width;

        // Headers
        if line.starts_with("#### ") {
            let content = &line[5..];
            let wrapped = wrap_line(&format!("\x1b[1;35m{}\x1b[0m", content), w, 0);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }
        if line.starts_with("### ") {
            let content = &line[4..];
            let wrapped = wrap_line(&format!("\x1b[1;35m{}\x1b[0m", content), w, 0);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }
        if line.starts_with("## ") {
            let content = &line[3..];
            let wrapped = wrap_line(&format!("\x1b[1;34m{}\x1b[0m", content), w, 0);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }
        if line.starts_with("# ") {
            let content = &line[2..];
            let wrapped = wrap_line(&format!("\x1b[1;33m{}\x1b[0m", content), w, 0);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }

        // Horizontal rule
        if line.trim() == "---" || line.trim() == "***" || line.trim() == "___" {
            let rule_width = w.min(40);
            let _ = write!(out, "\x1b[2m{}\x1b[0m{}", "─".repeat(rule_width), NL);
            return;
        }

        // Unordered list
        if line.starts_with("- ") || line.starts_with("* ") {
            let content = format_inline(&line[2..]);
            let prefix = "  \x1b[36m•\x1b[0m ";
            let prefix_display_width = 4;
            let wrapped = wrap_line(&format!("{}{}", prefix, content), w, prefix_display_width);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }

        // Indented list items
        if let Some(rest) = line.strip_prefix("  - ").or_else(|| line.strip_prefix("  * ")) {
            let content = format_inline(rest);
            let prefix = "    \x1b[36m◦\x1b[0m ";
            let prefix_display_width = 6;
            let wrapped = wrap_line(&format!("{}{}", prefix, content), w, prefix_display_width);
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }

        // Ordered list
        if let Some(dot_pos) = line.find(". ") {
            if dot_pos <= 3 && line[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
                let num = &line[..dot_pos];
                let content = format_inline(&line[dot_pos + 2..]);
                let prefix = format!("  \x1b[36m{}.\x1b[0m ", num);
                let prefix_display_width = 3 + num.len() + 1;
                let wrapped = wrap_line(&format!("{}{}", prefix, content), w, prefix_display_width);
                let _ = write!(out, "{}{}", wrapped, NL);
                return;
            }
        }

        // Blockquote
        if line.starts_with("> ") {
            let content = format_inline(&line[2..]);
            let prefix = "  \x1b[2m│\x1b[0m ";
            let prefix_display_width = 4;
            let wrapped = wrap_line(
                &format!("{}\x1b[3m{}\x1b[0m", prefix, content),
                w,
                prefix_display_width,
            );
            let _ = write!(out, "{}{}", wrapped, NL);
            return;
        }

        // Regular text
        let formatted = format_inline(line);
        let wrapped = wrap_line(&formatted, w, 0);
        let _ = write!(out, "{}{}", wrapped, NL);
    }

    fn render_code_block(&mut self) {
        let stderr = io::stderr();
        let mut out = stderr.lock();

        let lang = std::mem::take(&mut self.code_lang);
        let code = std::mem::take(&mut self.code_buf);
        self.in_code_block = false;

        let syntax = if !lang.is_empty() {
            self.syntax_set.find_syntax_by_token(&lang)
        } else {
            None
        };

        if !lang.is_empty() {
            let _ = write!(out, "\x1b[2;36m  ╭─ {}\x1b[0m{}", lang, NL);
        } else {
            let _ = write!(out, "\x1b[2;36m  ╭─\x1b[0m{}", NL);
        }

        if let Some(syntax) = syntax {
            let mut highlighter =
                syntect::easy::HighlightLines::new(syntax, &self.theme);
            for line in code.lines() {
                match highlighter.highlight_line(line, &self.syntax_set) {
                    Ok(ranges) => {
                        let _ = write!(out, "  \x1b[2m│\x1b[0m ");
                        for (style, text) in ranges {
                            let fg = style.foreground;
                            let _ = write!(
                                out,
                                "\x1b[38;2;{};{};{}m{}\x1b[0m",
                                fg.r, fg.g, fg.b, text
                            );
                        }
                        let _ = write!(out, "{}", NL);
                    }
                    Err(_) => {
                        let _ = write!(out, "  \x1b[2m│\x1b[0m {}{}", line, NL);
                    }
                }
            }
        } else {
            for line in code.lines() {
                let _ = write!(out, "  \x1b[2m│\x1b[0m \x1b[37m{}\x1b[0m{}", line, NL);
            }
        }

        let _ = write!(out, "\x1b[2;36m  ╰─\x1b[0m{}", NL);
    }
}

/// Apply inline markdown formatting to a line of text.
fn format_inline(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 32);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, &['*', '*']) {
                result.push_str("\x1b[1m");
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&inner);
                result.push_str("\x1b[0m");
                i = end + 2;
                continue;
            }
        }

        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if let Some(end) = find_single_closing(&chars, i + 1, '*') {
                result.push_str("\x1b[3m");
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&inner);
                result.push_str("\x1b[0m");
                i = end + 1;
                continue;
            }
        }

        if chars[i] == '`' {
            if let Some(end) = find_single_closing(&chars, i + 1, '`') {
                result.push_str("\x1b[36m");
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&inner);
                result.push_str("\x1b[0m");
                i = end + 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_inline_bold() {
        let result = format_inline("hello **world** end");
        assert!(result.contains("\x1b[1m"));
        assert!(result.contains("world"));
    }

    #[test]
    fn test_format_inline_code() {
        let result = format_inline("use `println!` here");
        assert!(result.contains("\x1b[36m"));
        assert!(result.contains("println!"));
    }

    #[test]
    fn test_format_inline_plain() {
        let result = format_inline("just plain text");
        assert_eq!(result, "just plain text");
    }

    #[test]
    fn test_display_width_ascii() {
        assert_eq!(display_width("hello"), 5);
    }

    #[test]
    fn test_display_width_cjk() {
        assert_eq!(display_width("你好"), 4);
    }

    #[test]
    fn test_display_width_ignores_ansi() {
        assert_eq!(display_width("\x1b[1mhello\x1b[0m"), 5);
        assert_eq!(display_width("\x1b[36m你好\x1b[0m"), 4);
    }

    #[test]
    fn test_wrap_line_no_wrap_needed() {
        let result = wrap_line("short", 80, 0);
        assert_eq!(result, "short");
    }

    #[test]
    fn test_wrap_line_wraps_long_ascii() {
        let text = "a".repeat(20);
        let result = wrap_line(&text, 10, 2);
        assert!(result.contains("\r\n"));
        let lines: Vec<&str> = result.split("\r\n").collect();
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_wrap_line_cjk() {
        let text = "你好世界啊";
        let result = wrap_line(text, 8, 0);
        assert!(result.contains("\r\n"));
    }

    #[test]
    fn test_wrap_line_with_ansi() {
        let text = "\x1b[1mhello world\x1b[0m";
        let result = wrap_line(text, 8, 0);
        assert!(result.contains("\r\n"));
        assert!(result.contains("\x1b[1m"));
    }
}
