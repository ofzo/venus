/// Text input state with cursor, history, and slash command completion.
pub struct InputState {
    pub buffer: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub history_working: String,
    /// Unified completion items driving the Claude-Code style popup.
    pub completion_items: Vec<CompletionItem>,
    pub completion_index: usize,
    /// Ghost text shown inline after cursor (dimmed preview of completion).
    pub ghost_text: Option<String>,
    /// Whether we're in file completion mode (after typing @).
    pub file_completion_active: bool,
}

/// One entry in the completion popup.
#[derive(Clone)]
pub struct CompletionItem {
    /// Left column: command (with leading `/`) or file path under the `@` token.
    pub label: String,
    /// Right column: short description, rendered right-aligned.
    pub description: String,
    /// Full buffer replacement applied when this item is accepted (Tab).
    pub accept: String,
    /// Per-char hit flags for `label`: `true` where the char matches the query.
    /// Slash commands mark a leading prefix run; `@` file completions mark a
    /// scattered subsequence (fzf-style fuzzy match).
    pub matched: Vec<bool>,
}

const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/exit",
    "/quit",
    "/clear",
    "/cost",
    "/model",
    "/history",
    "/diff",
    "/compact",
    "/config",
    "/doctor",
    "/context",
    "/tokens",
    "/sessions",
    "/resume",
    "/commit",
    "/review",
    "/init",
    "/memory",
    "/skills",
    "/tasks",
    "/plan",
    "/vim",
    "/effort",
    "/copy",
    "/version",
    "/status",
    "/summary",
    "/export",
    "/rewind",
    "/return",
    "/permissions",
    "/mcp",
    "/plugin",
    "/rename",
    "/hooks",
    "/delete-session",
    "/add-dir",
    "/fast",
    "/files",
    "/keybindings",
    "/color",
    "/theme",
    "/sandbox-toggle",
    "/stats",
    "/agents",
    "/output-style",
    "/branch",
    "/btw",
    "/tag",
    "/ps",
    "/attach",
    "/kill",
];

impl InputState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            history_working: String::new(),
            completion_items: Vec::new(),
            completion_index: 0,
            ghost_text: None,
            file_completion_active: false,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        // Completion state is recomputed by `update_completions`, which the
        // caller runs after any buffer mutation while in normal input mode.
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.buffer[..self.cursor_pos]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.buffer.drain(prev..self.cursor_pos);
            self.cursor_pos = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor_pos < self.buffer.len() {
            let ch = self.buffer[self.cursor_pos..].chars().next().unwrap();
            self.buffer
                .drain(self.cursor_pos..self.cursor_pos + ch.len_utf8());
        }
    }

    pub fn move_cursor_left(&mut self) {
        if let Some((i, _)) = self.buffer[..self.cursor_pos].char_indices().next_back() {
            self.cursor_pos = i;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(ch) = self.buffer[self.cursor_pos..].chars().next() {
            self.cursor_pos += ch.len_utf8();
        }
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.buffer.len();
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor_pos = 0;
        self.clear_completions();
    }

    /// Delete from cursor to end of line.
    pub fn delete_to_end(&mut self) {
        self.buffer.truncate(self.cursor_pos);
    }

    /// Delete from cursor to start of line.
    pub fn delete_to_start(&mut self) {
        self.buffer.drain(..self.cursor_pos);
        self.cursor_pos = 0;
    }

    /// Delete word backward from cursor.
    pub fn delete_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        // Skip trailing whitespace
        let mut pos = self.cursor_pos;
        while pos > 0 && self.buffer[..pos].ends_with(' ') {
            pos -= 1;
        }
        // Find word boundary
        while pos > 0 && !self.buffer[..pos].ends_with(' ') {
            pos -= 1;
        }
        self.buffer.drain(pos..self.cursor_pos);
        self.cursor_pos = pos;
    }

    /// Take the buffer content, push to history, reset state.
    pub fn take_buffer(&mut self) -> String {
        let buf = std::mem::take(&mut self.buffer);
        self.cursor_pos = 0;
        if !buf.trim().is_empty() {
            self.history.push(buf.clone());
        }
        self.history_index = None;
        self.clear_completions();
        buf
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        if self.history_index.is_none() {
            self.history_working = self.buffer.clone();
        }
        let idx = self.history_index.unwrap_or(self.history.len());
        if idx > 0 {
            let new_idx = idx - 1;
            self.buffer = self.history[new_idx].clone();
            self.cursor_pos = self.buffer.len();
            self.history_index = Some(new_idx);
        }
    }

    pub fn history_down(&mut self) {
        if let Some(idx) = self.history_index {
            if idx + 1 < self.history.len() {
                let new_idx = idx + 1;
                self.buffer = self.history[new_idx].clone();
                self.cursor_pos = self.buffer.len();
                self.history_index = Some(new_idx);
            } else {
                self.buffer = self.history_working.clone();
                self.cursor_pos = self.buffer.len();
                self.history_index = None;
            }
        }
    }

    /// Recompute completion items from the current buffer state. Call after any
    /// buffer mutation in normal input mode.
    pub fn update_completions(&mut self, working_dir: &std::path::Path) {
        if self.buffer.contains('\n') {
            self.clear_completions();
            return;
        }
        if self.buffer.starts_with('/') {
            self.file_completion_active = false;
            self.complete_slash();
            return;
        }
        if let Some(at_pos) = self.buffer.rfind('@') {
            let token = &self.buffer[at_pos + 1..];
            if !token.contains(char::is_whitespace) {
                self.file_completion_active = true;
                self.complete_file_path(working_dir);
                return;
            }
        }
        self.clear_completions();
    }

    /// Populate slash-command completion items matching the buffer prefix.
    pub fn complete_slash(&mut self) {
        if !self.buffer.starts_with('/') {
            self.completion_items.clear();
            return;
        }
        let q = self.buffer.clone();
        let qc = q.chars().count();
        let mut hits: Vec<&str> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(q.as_str()))
            .copied()
            .collect();
        hits.sort_unstable();
        self.completion_items = hits
            .into_iter()
            .take(8)
            .map(|cmd| {
                let mut matched = vec![false; cmd.chars().count()];
                let ml = qc.min(matched.len());
                matched[..ml].fill(true);
                CompletionItem {
                    label: cmd.to_string(),
                    description: slash_description(cmd).to_string(),
                    accept: cmd.to_string(),
                    matched,
                }
            })
            .collect();
        self.completion_index = 0;
        self.update_ghost_text();
    }

    /// Populate file-path completion items for the active `@` token using
    /// fzf-style fuzzy subsequence matching (smart-case) with score ranking.
    /// The `@` token may target the whole working directory (`@lib` matches
    /// `src/lib.rs` deep in the tree) or be scoped to a subtree with a path
    /// prefix (`@src/li` searches under `src/`). Results include both files
    /// and directories anywhere under the (possibly scoped) search root, so
    /// subdirectories and their contents are selectable, not just the flat
    /// top-level listing. Hit highlighting marks the scattered query chars at
    /// their position within the displayed (full relative) label.
    pub fn complete_file_path(&mut self, working_dir: &std::path::Path) {
        if !self.file_completion_active {
            return;
        }
        let at_pos = self.buffer.rfind('@').unwrap_or(0);
        let before_at = self.buffer[..at_pos].to_string();
        let prefix = self.buffer[at_pos + 1..].to_string();

        // Split the typed `@` token into a directory part and a name query.
        let (dir_prefix, name_query) = match prefix.rsplit_once('/') {
            Some((d, f)) => (d.to_string(), f.to_string()),
            None => (String::new(), prefix.clone()),
        };
        let dir_prefix_disp = if dir_prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", dir_prefix)
        };
        let search_dir = if dir_prefix.is_empty() {
            working_dir.to_path_buf()
        } else {
            working_dir.join(&dir_prefix)
        };
        let dir_offset = dir_prefix_disp.chars().count();

        // Recursive walk: bound the work so big trees stay responsive. The
        // query matches against each entry's relative path, so deep files and
        // nested directories are selectable just like top-level entries.
        let visited = collect_entries_recursively(&search_dir, &name_query, 4096);

        let mut scored: Vec<(FuzzyMatch, String, bool)> = visited
            .into_iter()
            .filter_map(|(rel, is_dir)| fuzzy_match(&name_query, &rel).map(|fm| (fm, rel, is_dir)))
            .collect();
        // Highest score first (Ord: best compares as Greatest).
        scored.sort_by(|a, b| b.0.score.cmp(&a.0.score));

        let items: Vec<CompletionItem> = scored
            .into_iter()
            .take(8)
            .map(|(fm, rel, is_dir)| {
                let path = format!("{}{}", dir_prefix_disp, rel);
                let desc = if is_dir { "directory" } else { "file" };
                // Build per-char hit flags across the full displayed label,
                // marking the matched rel-path chars at the directory offset
                // so the directory prefix stays grey.
                let mut matched = vec![false; path.chars().count()];
                for &hi in &fm.hits {
                    let idx = dir_offset + hi;
                    if idx < matched.len() {
                        matched[idx] = true;
                    }
                }
                CompletionItem {
                    label: path.clone(),
                    description: desc.to_string(),
                    accept: format!("{}@{}", before_at, path),
                    matched,
                }
            })
            .collect();

        self.completion_items = items;
        self.completion_index = 0;
        // Ghost text: preview the un-typed tail of the best match, but only
        // when it is a genuine prefix match (fzf fuzzy hits do not preview).
        if let Some(first) = self.completion_items.first() {
            self.ghost_text = None;
            if !name_query.is_empty() {
                // Byte offset of the char at `dir_offset` (char count) so the
                // slice below is char-boundary safe for multibyte names too.
                let byte_off = first
                    .label
                    .char_indices()
                    .nth(dir_offset)
                    .map(|(i, _)| i)
                    .unwrap_or(first.label.len());
                let sub = &first.label[byte_off..];
                if sub.starts_with(name_query.as_str()) && sub.len() > name_query.len() {
                    self.ghost_text = Some(sub[name_query.len()..].to_string());
                }
            }
        } else {
            self.ghost_text = None;
        }
    }

    /// Update inline ghost text for the leading slash-completion match.
    fn update_ghost_text(&mut self) {
        self.ghost_text = None;
        if !self.buffer.starts_with('/') {
            return;
        }
        if let Some(first) = self.completion_items.first() {
            if first.label.starts_with(&self.buffer) && first.label.len() > self.buffer.len() {
                self.ghost_text = Some(first.label[self.buffer.len()..].to_string());
            }
        }
    }

    /// Move the completion selection by `delta`, wrapping within the list.
    pub fn move_completion(&mut self, delta: i32) {
        if self.completion_items.is_empty() {
            return;
        }
        let n = self.completion_items.len() as i32;
        let mut i = self.completion_index as i32 + delta;
        if i < 0 {
            i += n;
        }
        self.completion_index = (i % n) as usize;
    }

    /// Accept the currently selected completion item into the buffer.
    pub fn accept_completion(&mut self) {
        if let Some(item) = self.completion_items.get(self.completion_index) {
            self.buffer = item.accept.clone();
            self.cursor_pos = self.buffer.len();
        }
        self.clear_completions();
    }

    pub fn clear_completions(&mut self) {
        self.completion_items.clear();
        self.completion_index = 0;
        self.ghost_text = None;
        self.file_completion_active = false;
    }
}

use std::cmp::Ordering;

/// Result of a successful fzf-style fuzzy subsequence match of `query`
/// against `label`.
pub struct FuzzyMatch {
    /// Char indices (into `label`) that the query chars hit, in query order.
    pub hits: Vec<usize>,
    /// Ranking score; a later `Ord` value means a better match.
    pub score: FuzzyScore,
}

/// Ranking score for a fuzzy match. Best wins; `Ord` compares fields in the
/// order listed below (tie-breakers applied left to right).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyScore {
    /// Pure prefix match (the query starts at `label[0]`).
    pub prefix: bool,
    /// Length of the longest consecutive run of hit chars.
    pub max_run: usize,
    /// Number of separate (non-adjacent) hit runs.
    pub runs: usize,
    /// Index of the first hit char (smaller = earlier = better).
    pub start: usize,
    /// Length of the label (shorter = better tie-breaker).
    pub label_len: usize,
    /// Label text, for stable alphabetical tie-breaking.
    pub label: String,
}

impl Ord for FuzzyScore {
    fn cmp(&self, other: &Self) -> Ordering {
        // Best = Greatest. `prefix`/`max_run`: larger is better -> self vs
        // other. `runs`/`start`/`label_len`/`label`: smaller is better -> the
        // comparison is flipped so the smaller side yields Greater.
        self.prefix
            .cmp(&other.prefix)
            .then(self.max_run.cmp(&other.max_run))
            .then(other.runs.cmp(&self.runs))
            .then(other.start.cmp(&self.start))
            .then(other.label_len.cmp(&self.label_len))
            .then(other.label.cmp(&self.label))
    }
}

impl PartialOrd for FuzzyScore {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// fzf-style subsequence match with smart-case and scoring. Returns `Some`
/// iff `query` is a subsequence of `label`. Case sensitivity is "smart":
/// the match is case-sensitive when the query contains any uppercase letter,
/// and case-insensitive otherwise. An empty query matches everything (no
/// highlighting). The greedy match consumes label chars left-to-right.
pub fn fuzzy_match(query: &str, label: &str) -> Option<FuzzyMatch> {
    let q_chars: Vec<char> = query.chars().collect();
    let l_chars: Vec<char> = label.chars().collect();

    if q_chars.is_empty() {
        return Some(FuzzyMatch {
            hits: Vec::new(),
            score: FuzzyScore {
                prefix: true,
                max_run: 0,
                runs: 0,
                start: 0,
                label_len: l_chars.len(),
                label: label.to_string(),
            },
        });
    }

    let has_upper = q_chars.iter().any(|c| c.is_uppercase());
    let eq = |qc: char, lc: char| {
        if has_upper {
            qc == lc
        } else {
            qc.eq_ignore_ascii_case(&lc)
        }
    };

    let mut hits: Vec<usize> = Vec::with_capacity(q_chars.len());
    let mut li = 0usize;
    for &qc in &q_chars {
        let mut found = false;
        while li < l_chars.len() {
            if eq(qc, l_chars[li]) {
                hits.push(li);
                li += 1;
                found = true;
                break;
            }
            li += 1;
        }
        if !found {
            return None;
        }
    }

    let prefix = hits[0] == 0;
    let mut max_run = 1usize;
    let mut runs = 1usize;
    let mut run = 1usize;
    for w in 1..hits.len() {
        if hits[w] == hits[w - 1] + 1 {
            run += 1;
            max_run = max_run.max(run);
        } else {
            runs += 1;
            run = 1;
        }
    }
    let start = hits[0];

    Some(FuzzyMatch {
        hits,
        score: FuzzyScore {
            prefix,
            max_run,
            runs,
            start,
            label_len: l_chars.len(),
            label: label.to_string(),
        },
    })
}

/// Directory trees that are skipped entirely during the recursive `@` file
/// walk so completion stays responsive and free of build/VCS noise.
const FAT_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    ".parcel-cache",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    "deps",
    "_build",
];

/// Bounded depth-first walk of `root`. Yields `(relative_path, is_dir)`
/// entries whose paths are relative to `root`, so a `@` token scoped to a
/// subdirectory still surfaces nested files and directories inside it.
/// Hidden (`.`-prefixed) entries are excluded unless the query starts with a
/// dot, and the [`FAT_SKIP_DIRS`] trees are pruned entirely. At most `cap`
/// entries are collected to keep the popup responsive.
fn collect_entries_recursively(
    root: &std::path::Path,
    name_query: &str,
    cap: usize,
) -> Vec<(String, bool)> {
    use std::collections::VecDeque;
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut stack: VecDeque<(std::path::PathBuf, String)> = VecDeque::new();
    stack.push_back((root.to_path_buf(), String::new()));
    let dot_query = name_query.starts_with('.');
    while let Some((dir, rel_prefix)) = stack.pop_back() {
        if out.len() >= cap {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut items: Vec<std::fs::DirEntry> = entries.filter_map(|e| e.ok()).collect();
        items.sort_by_key(|e| e.file_name());
        for e in items {
            if out.len() >= cap {
                break;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.is_empty() || name == ".DS_Store" {
                continue;
            }
            if name.starts_with('.') && !dot_query {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && FAT_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let rel = if rel_prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", rel_prefix, name)
            };
            out.push((rel.clone(), is_dir));
            if is_dir {
                stack.push_back((dir.join(&name), rel));
            }
        }
    }
    out
}

/// Short, human-readable description for a slash command (right column).
fn slash_description(cmd: &str) -> &'static str {
    match cmd {
        "/help" => "Show help",
        "/exit" | "/quit" => "Quit Venus",
        "/clear" => "Clear conversation",
        "/cost" => "Show token costs",
        "/model" => "Switch model",
        "/history" => "Show input history",
        "/diff" => "Show uncommitted diff",
        "/compact" => "Compact context",
        "/config" => "Edit configuration",
        "/doctor" => "Diagnose setup",
        "/context" => "Show context usage",
        "/tokens" => "Show token usage",
        "/sessions" => "List sessions",
        "/resume" => "Resume a session",
        "/commit" => "Create a git commit",
        "/review" => "Request code review",
        "/init" => "Create VENUS.md",
        "/memory" => "Edit memory",
        "/skills" => "List skills",
        "/tasks" => "List tasks",
        "/plan" => "Plan mode",
        "/vim" => "Toggle vim mode",
        "/effort" => "Set reasoning effort",
        "/copy" => "Copy last reply",
        "/version" => "Show version",
        "/status" => "Show status",
        "/summary" => "Summarize chat",
        "/export" => "Export conversation",
        "/rewind" => "Rewind state",
        "/return" => "Rewind to before #N (last if omitted)",
        "/permissions" => "Edit permissions",
        "/mcp" => "List MCP servers",
        "/plugin" => "Manage plugins",
        "/rename" => "Rename session",
        "/hooks" => "Edit hooks",
        "/delete-session" => "Delete session",
        "/add-dir" => "Add a directory",
        "/fast" => "Fast mode",
        "/files" => "List files",
        "/keybindings" => "Show keybindings",
        "/color" => "Toggle color",
        "/theme" => "Set theme",
        "/sandbox-toggle" => "Toggle sandbox",
        "/stats" => "Show stats",
        "/agents" => "Manage agents",
        "/output-style" => "Set output style",
        "/branch" => "Git branch",
        "/btw" => "Notes",
        "/tag" => "Tag a point",
        "/ps" => "List processes",
        "/attach" => "Attach to session",
        "/kill" => "Kill a process",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(buf: &str) -> InputState {
        let mut s = InputState::new();
        s.buffer = buf.to_string();
        s.cursor_pos = s.buffer.len();
        s
    }

    #[test]
    fn complete_slash_matches_prefix_and_describes() {
        let mut s = state_with("/c");
        s.complete_slash();
        // All top-level matches start with the typed prefix.
        assert!(s.completion_items.iter().all(|i| i.label.starts_with("/c")));
        // The curated description is wired up.
        let compact = s.completion_items.iter().find(|i| i.label == "/compact");
        assert_eq!(compact.unwrap().description, "Compact context");
        // The typed prefix (`/c`, 2 chars) is flagged as matched per char.
        assert!(s.completion_items.iter().all(|i| {
            i.matched.len() >= 2 && i.matched[0] && i.matched[1] && !i.matched[2..].contains(&true)
        }));
        // Popup is capped for display and never exceeds 8 rows.
        assert!(s.completion_items.len() <= 8);
    }

    #[test]
    fn move_completion_wraps() {
        let mut s = state_with("/e");
        s.complete_slash();
        let n = s.completion_items.len();
        assert!(n >= 2, "need >=2 matches to wrap");
        // Down from the last item wraps to the first.
        s.completion_index = n - 1;
        s.move_completion(1);
        assert_eq!(s.completion_index, 0);
        // Up from the first wraps to the last.
        s.move_completion(-1);
        assert_eq!(s.completion_index, n - 1);
    }

    #[test]
    fn accept_completion_fills_selected_item() {
        let mut s = state_with("/comp");
        s.complete_slash();
        // Walk to the second match and accept it.
        s.move_completion(1);
        let accepted = s.completion_items[s.completion_index].accept.clone();
        s.accept_completion();
        assert_eq!(s.buffer, accepted);
        assert!(s.completion_items.is_empty(), "popup closes on accept");
    }

    #[test]
    fn empty_completion_is_noop() {
        let mut s = state_with("zzz");
        s.complete_slash();
        assert!(s.completion_items.is_empty());
        s.move_completion(1); // must not panic with empty list
    }

    #[test]
    fn fuzzy_match_subsequence_and_hit_offsets() {
        // "lib" is a scattered subsequence of "src_lib.rs" -> hits at 4,5,6.
        let fm = fuzzy_match("lib", "src_lib.rs").expect("lib matches src_lib.rs");
        assert_eq!(fm.hits, vec![4, 5, 6]);
        // (max_run 3, runs 1, start 4)
        assert!(fm.score.max_run >= 3);
    }

    #[test]
    fn fuzzy_match_non_match_returns_none() {
        assert!(fuzzy_match("xy", "src_lib.rs").is_none());
    }

    #[test]
    fn fuzzy_match_smart_case_lowercase_query_matches_uppercase() {
        // All-lowercase query is case-insensitive.
        assert!(fuzzy_match("readme", "README.md").is_some());
        // An uppercase query char forces case sensitivity.
        assert!(fuzzy_match("README", "README.md").is_some());
        assert!(fuzzy_match("ReAdMe", "readme").is_none());
    }

    #[test]
    fn fuzzy_match_empty_query_matches_all() {
        let fm = fuzzy_match("", "anything").expect("empty query matches");
        assert!(fm.hits.is_empty());
        assert!(fm.score.prefix);
    }

    #[test]
    fn fuzzy_score_prefix_beats_infix() {
        let prefix = fuzzy_match("lib", "lib.rs").unwrap().score;
        let infix = fuzzy_match("lib", "xlib.rs").unwrap().score;
        // Prefix match should sort after (be greater than) an infix match.
        assert!(prefix > infix);
        assert!(prefix.prefix);
        assert!(!infix.prefix);
    }

    #[test]
    fn fuzzy_score_longer_run_beats_shorter() {
        // "ab" in "abcd" is one run of length 2 (prefix).
        let long = fuzzy_match("ab", "abcd").unwrap().score;
        // "ab" in "axb" is two 1-runs.
        let short = fuzzy_match("ab", "axb").unwrap().score;
        assert!(long > short);
        assert!(long.max_run > short.max_run);
    }

    /// Regression: `@` must surface files and directories inside nested
    /// subdirectories, not just the flat top-level listing.
    #[test]
    fn collect_entries_recursively_descends_into_subdirs() {
        let root = std::env::temp_dir().join(format!(
            "venus_fuzzy_walk_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(root.join("nested").join("deeper"));
        std::fs::write(root.join("top.txt"), b"").unwrap();
        std::fs::write(root.join("nested").join("lib.rs"), b"").unwrap();
        std::fs::write(root.join("nested").join("deeper").join("deep.txt"), b"").unwrap();

        let entries = collect_entries_recursively(&root, "", 4096);
        let labels: Vec<&String> = entries.iter().map(|(r, _)| r).collect();
        // A file two levels deep is selectable.
        assert!(labels
            .iter()
            .any(|r| r.as_str() == "nested/deeper/deep.txt"));
        // A file one level deep is selectable.
        assert!(labels.iter().any(|r| r.as_str() == "nested/lib.rs"));
        // The top-level file is there too.
        assert!(labels.iter().any(|r| r.as_str() == "top.txt"));
        // Subdirectories appear as directory entries so they are navigable.
        assert!(entries.iter().any(|(r, d)| r == "nested" && *d));

        std::fs::remove_dir_all(&root).ok();
    }
}
