//! Structured diff computation for file-write/edit tool results.
//!
//! The tool layer (`FileWriteTool` / `FileEditTool`) captures the file's
//! content *before* it is overwritten, computes a line-level diff via the
//! `similar` crate, and attaches the resulting [`ToolDiff`] to the
//! [`crate::tool::ToolResult`] it returns. The TUI renderer then colourises
//! the structured lines (added = green, removed = red, context = dim) instead
//! of having to re-parse a unified-diff string blob.
//!
//! The diff is grouped into hunks (3 lines of context by default, matching the
//! unified-diff convention) so a small change to a huge file renders compactly.

use similar::{ChangeTag, TextDiff};

/// Kind of a single rendered diff line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// A `@@ -a,b +c,d @@` hunk header.
    HunkHeader,
    /// An unchanged context line.
    Context,
    /// A line present only in the new content (`+`).
    Add,
    /// A line present only in the old content (`-`).
    Remove,
}

/// A single rendered diff line: its kind + the (already-prefixed / raw) text.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// Raw line value (no `+`/`-`/` ` prefix; the renderer applies the sign).
    pub text: String,
    /// 1-based old-file line number. `None` for Add lines, hunk headers, and
    /// the all-add `compute_new_file_diff` path.
    pub old_ln: Option<usize>,
    /// 1-based new-file line number. `None` for Remove lines and hunk
    /// headers.
    pub new_ln: Option<usize>,
}

/// A structured, renderable diff attached to a `ToolResult`.
#[derive(Debug, Clone)]
pub struct ToolDiff {
    /// Display path of the file that was written/edited.
    pub path: String,
    /// Ordered rendered lines (hunk headers + +/- / context lines).
    pub lines: Vec<DiffLine>,
}

impl ToolDiff {
    pub fn new(path: impl Into<String>, lines: Vec<DiffLine>) -> Self {
        Self {
            path: path.into(),
            lines,
        }
    }
}

/// Compute a unified-style, structured, line-level diff between `old` and
/// `new`.
///
/// Returns ordered `DiffLine`s grouped into hunks with 3 lines of context.
/// Newline stripping: `similar`'s `from_lines` keeps the trailing `\n` on each
/// line, so we strip it for clean rendering.
pub fn compute_file_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);

    let mut out: Vec<DiffLine> = Vec::new();
    for group in diff.grouped_ops(3) {
        if group.is_empty() {
            continue;
        }

        // Compute the hunk header ranges (`@@ -a,b +c,d @@`) by spanning the
        // whole group: `similar` returns ops within a group in increasing
        // position, so the hunk's old/new range is `[first.start .. last.end)`.
        let (old_start, old_len) = group_span(
            group.first().map(|o| o.old_range().clone()),
            group.last().map(|o| o.old_range().clone()),
        );
        let (new_start, new_len) = group_span(
            group.first().map(|o| o.new_range().clone()),
            group.last().map(|o| o.new_range().clone()),
        );

        out.push(DiffLine {
            kind: DiffLineKind::HunkHeader,
            text: format_hunk_header(old_start, old_len, new_start, new_len),
            old_ln: None,
            new_ln: None,
        });

        for op in group {
            for change in diff.iter_changes(&op) {
                let value = strip_trailing_newline(change.value());
                // similar line indices are 0-based; convert to 1-based for
                // display in the gutter.
                let old_ln = change.old_index().map(|i| i + 1);
                let new_ln = change.new_index().map(|i| i + 1);
                match change.tag() {
                    ChangeTag::Equal => out.push(DiffLine {
                        kind: DiffLineKind::Context,
                        text: value.to_string(),
                        old_ln,
                        new_ln,
                    }),
                    ChangeTag::Delete => out.push(DiffLine {
                        kind: DiffLineKind::Remove,
                        text: value.to_string(),
                        old_ln,
                        new_ln,
                    }),
                    ChangeTag::Insert => out.push(DiffLine {
                        kind: DiffLineKind::Add,
                        text: value.to_string(),
                        old_ln,
                        new_ln,
                    }),
                }
            }
        }
    }

    out
}

/// Build a diff representing a freshly-created file: a single all-add "hunk"
/// with no removed lines. Produces a leading `@@ -0,0 +1,N @@` header followed
/// by `N` add lines.
pub fn compute_new_file_diff(new: &str) -> Vec<DiffLine> {
    let count = new.lines().count();
    let mut out: Vec<DiffLine> = Vec::with_capacity(count + 1);
    out.push(DiffLine {
        kind: DiffLineKind::HunkHeader,
        text: format_hunk_header(0, 0, 1, count),
        old_ln: None,
        new_ln: None,
    });
    for (i, line) in new.lines().enumerate() {
        out.push(DiffLine {
            kind: DiffLineKind::Add,
            text: line.to_string(),
            old_ln: None,
            new_ln: Some(i + 1),
        });
    }
    out
}

/// Span of a hunk over an op group: 1-based `start` + count of lines, where
/// `first` and `last` are the first/last ops' old (or new) ranges. Ops within a
/// single group are contiguous and increasing in position, so the span is
/// `[first.start .. last.end)`.
fn group_span(
    first: Option<std::ops::Range<usize>>,
    last: Option<std::ops::Range<usize>>,
) -> (usize, usize) {
    match (first, last) {
        (Some(f), Some(l)) => {
            let start_0 = f.start.min(l.start);
            let end_0 = f.end.max(l.end);
            (start_0 + 1, end_0.saturating_sub(start_0))
        }
        _ => (1, 0),
    }
}

fn format_hunk_header(old_start: usize, old_len: usize, new_start: usize, new_len: usize) -> String {
    // GNU unified-diff convention: a 0-length range is written at the line
    // immediately before the insertion/deletion (`start-1,0`). For length 1 we
    // still print `start,1` for clarity rather than the collapsed `start` form.
    let old_part = if old_len == 0 {
        format!("{},0", old_start.saturating_sub(1))
    } else {
        format!("{},{}", old_start, old_len)
    };
    let new_part = if new_len == 0 {
        format!("{},0", new_start.saturating_sub(1))
    } else {
        format!("{},{}", new_start, new_len)
    };
    format!("@@ -{} +{} @@", old_part, new_part)
}

fn strip_trailing_newline(s: &str) -> &str {
    s.strip_suffix('\n')
        .or_else(|| s.strip_suffix("\r\n"))
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_replace() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let lines = compute_file_diff(old, new);
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::Remove && l.text == "b"));
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::Add && l.text == "B"));
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::Context && l.text == "a"));
        assert!(lines.iter().any(|l| l.kind == DiffLineKind::HunkHeader));
    }

    #[test]
    fn test_new_file_all_add() {
        let new = "x\ny\nz\n";
        let lines = compute_new_file_diff(new);
        assert_eq!(lines.len(), 4);
        assert!(matches!(lines[0].kind, DiffLineKind::HunkHeader));
        for l in &lines[1..] {
            assert_eq!(l.kind, DiffLineKind::Add);
        }
    }

    #[test]
    fn test_no_changes_is_empty() {
        let old = "a\nb\n";
        let new = "a\nb\n";
        let lines = compute_file_diff(old, new);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_line_numbers_populated() {
        // old: a b c d    ;  new: a B c d
        // expected: ctx(1,1) a, rem(2,-) b, add(-,2) B, ctx(3,3) c, ctx(4,4) d
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nd\n";
        let lines = compute_file_diff(old, new);
        // first non-header context line "a": (1,1)
        let ctx_a = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Context && l.text == "a")
            .expect("context a");
        assert_eq!(ctx_a.old_ln, Some(1));
        assert_eq!(ctx_a.new_ln, Some(1));
        // removed "b": old=2, new=None
        let rem_b = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Remove && l.text == "b")
            .expect("remove b");
        assert_eq!(rem_b.old_ln, Some(2));
        assert_eq!(rem_b.new_ln, None);
        // added "B": old=None, new=2
        let add_b = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Add && l.text == "B")
            .expect("add B");
        assert_eq!(add_b.old_ln, None);
        assert_eq!(add_b.new_ln, Some(2));
        // hunk headers carry no line numbers
        for l in lines.iter().filter(|l| l.kind == DiffLineKind::HunkHeader) {
            assert!(l.old_ln.is_none());
            assert!(l.new_ln.is_none());
        }
    }

    #[test]
    fn test_new_file_line_numbers() {
        let new = "x\ny\n";
        let lines = compute_new_file_diff(new);
        let adds: Vec<_> = lines.iter().filter(|l| l.kind == DiffLineKind::Add).collect();
        assert_eq!(adds.len(), 2);
        assert_eq!(adds[0].old_ln, None);
        assert_eq!(adds[0].new_ln, Some(1));
        assert_eq!(adds[1].old_ln, None);
        assert_eq!(adds[1].new_ln, Some(2));
    }

}
