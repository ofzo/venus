/// Text input state with cursor, history, and slash command completion.
pub struct InputState {
    pub buffer: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub history_working: String,
    pub completion_matches: Vec<String>,
    pub completion_index: usize,
}

const SLASH_COMMANDS: &[&str] = &[
    "/help", "/exit", "/quit", "/clear", "/cost", "/model", "/history", "/diff", "/compact",
    "/config", "/doctor", "/context", "/tokens", "/sessions", "/resume", "/commit", "/review",
    "/init", "/memory", "/skills", "/tasks", "/plan", "/vim", "/effort", "/copy", "/version",
    "/status", "/summary", "/export", "/rewind", "/permissions", "/mcp", "/plugin", "/rename",
    "/hooks", "/delete-session", "/add-dir", "/fast", "/files", "/keybindings", "/color", "/theme",
    "/sandbox-toggle", "/stats", "/agents", "/output-style", "/branch", "/btw", "/tag", "/ps",
    "/attach", "/kill",
];

impl InputState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_index: None,
            history_working: String::new(),
            completion_matches: Vec::new(),
            completion_index: 0,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
        self.clear_completions();
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

    /// Complete slash commands matching current buffer prefix.
    pub fn complete_slash(&mut self) {
        if !self.buffer.starts_with('/') {
            return;
        }
        self.completion_matches = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(&self.buffer))
            .map(|s| s.to_string())
            .collect();
        self.completion_index = 0;
    }

    /// Accept the current completion match.
    pub fn accept_completion(&mut self) {
        if let Some(match_) = self.completion_matches.get(self.completion_index) {
            self.buffer = match_.clone();
            self.cursor_pos = self.buffer.len();
        }
        self.clear_completions();
    }

    fn clear_completions(&mut self) {
        self.completion_matches.clear();
        self.completion_index = 0;
    }
}
