use std::path::PathBuf;

use reedline::{
    default_emacs_keybindings, default_vi_insert_keybindings, default_vi_normal_keybindings,
    ColumnarMenu, Completer, DefaultPrompt, DefaultPromptSegment,
    EditCommand, Emacs, FileBackedHistory, KeyCode, KeyModifiers, MenuBuilder, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, Span, Suggestion, Vi,
};

/// Slash commands available for tab completion.
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

/// Map color name to ANSI escape code.
fn color_ansi(name: &str) -> &str {
    match name {
        "blue" => "\x1b[34m",
        "green" => "\x1b[32m",
        "red" => "\x1b[31m",
        "yellow" => "\x1b[33m",
        "cyan" => "\x1b[36m",
        "magenta" => "\x1b[35m",
        "white" => "\x1b[37m",
        _ => "\x1b[36m", // default cyan
    }
}

/// A REPL input handler backed by reedline.
pub struct InputEditor {
    editor: Reedline,
    prompt_color: String,
    vim_mode: bool,
    history_path: Option<PathBuf>,
}

impl InputEditor {
    pub fn new(history_path: Option<PathBuf>, prompt_color: &str) -> Self {
        let history = history_path
            .clone()
            .and_then(|p| {
                // Ensure parent directory exists
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                FileBackedHistory::with_file(1000, p).ok()
            });

        let completer = Box::new(SlashCompleter);

        let completion_menu = Box::new(
            ColumnarMenu::default()
                .with_name("completion_menu")
                .with_columns(4),
        );

        // Set up keybindings with completion menu
        let mut keybindings = default_emacs_keybindings();
        keybindings.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu("completion_menu".to_string()),
                ReedlineEvent::MenuNext,
            ]),
        );
        // Shift+Enter or Alt+Enter for newline in multi-line input
        keybindings.add_binding(
            KeyModifiers::ALT,
            KeyCode::Enter,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );
        // Shift+Tab for permission mode cycling
        keybindings.add_binding(
            KeyModifiers::SHIFT,
            KeyCode::Tab,
            ReedlineEvent::Edit(vec![EditCommand::InsertString("\x00PERM_CYCLE\x00".to_string())]),
        );

        let edit_mode = Box::new(Emacs::new(keybindings));

        let mut editor = Reedline::create()
            .with_completer(completer)
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
            .with_edit_mode(edit_mode);

        if let Some(history) = history {
            editor = editor.with_history(Box::new(history));
        }

        Self { editor, prompt_color: prompt_color.to_string(), vim_mode: false, history_path }
    }

    /// Toggle between Emacs and Vi edit modes.
    pub fn toggle_vim_mode(&mut self) -> bool {
        self.vim_mode = !self.vim_mode;
        // Rebuild the editor with the new edit mode
        *self = InputEditor::new(self.history_path.clone(), &self.prompt_color);
        self.vim_mode = !self.vim_mode; // set to desired state
        // If vim was just toggled ON, rebuild again with vim
        if self.vim_mode {
            let completer = Box::new(SlashCompleter);
            let completion_menu = Box::new(
                ColumnarMenu::default()
                    .with_name("completion_menu")
                    .with_columns(4),
            );
            let mut insert_keybindings = default_vi_insert_keybindings();
            // Shift+Tab for permission mode cycling (vi insert mode)
            insert_keybindings.add_binding(
                KeyModifiers::SHIFT,
                KeyCode::Tab,
                ReedlineEvent::Edit(vec![EditCommand::InsertString("\x00PERM_CYCLE\x00".to_string())]),
            );
            let normal_keybindings = default_vi_normal_keybindings();
            let edit_mode = Box::new(Vi::new(insert_keybindings, normal_keybindings));

            let history = self.history_path
                .clone()
                .and_then(|p| FileBackedHistory::with_file(1000, p).ok());

            let mut editor = Reedline::create()
                .with_completer(completer)
                .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
                .with_edit_mode(edit_mode);

            if let Some(history) = history {
                editor = editor.with_history(Box::new(history));
            }

            self.editor = editor;
        }
        self.vim_mode
    }

    /// Check if vim mode is active.
    pub fn is_vim_mode(&self) -> bool {
        self.vim_mode
    }

    /// Sentinel string returned when Shift+Tab is pressed (permission mode cycle).
    pub const PERM_CYCLE_SENTINEL: &str = "\x00PERM_CYCLE\x00";

    /// Read a line of input with no status. Returns None on Ctrl+D (EOF).
    pub fn read_line(&mut self) -> Option<String> {
        self.read_line_with_status("")
    }

    /// Read a line of input. Returns None on Ctrl+D (EOF).
    /// `right_status` is displayed in the right prompt (e.g., model, cost).
    pub fn read_line_with_status(&mut self, right_status: &str) -> Option<String> {
        let color = color_ansi(&self.prompt_color);
        let prompt_str = format!("{}>\x1b[0m", color);
        let prompt = if right_status.is_empty() {
            DefaultPrompt::new(
                DefaultPromptSegment::Basic(prompt_str),
                DefaultPromptSegment::Empty,
            )
        } else {
            DefaultPrompt::new(
                DefaultPromptSegment::Basic(prompt_str),
                DefaultPromptSegment::Basic(right_status.to_string()),
            )
        };
        match self.editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                // Check for Shift+Tab permission cycle sentinel
                if line.contains(Self::PERM_CYCLE_SENTINEL) {
                    return Some(Self::PERM_CYCLE_SENTINEL.to_string());
                }
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    Some(String::new())
                } else {
                    Some(trimmed)
                }
            }
            Ok(Signal::CtrlC) => {
                // Ctrl+C clears current input, returns empty to continue loop
                Some(String::new())
            }
            Ok(Signal::CtrlD) => None,
            Err(_) => None,
        }
    }
}

/// Tab completer for slash commands.
struct SlashCompleter;

impl Completer for SlashCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        // Only complete if we're at the start of input with a slash
        let input = &line[..pos];
        if !input.starts_with('/') {
            return vec![];
        }

        SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(input))
            .map(|cmd| Suggestion {
                value: cmd.to_string(),
                description: None,
                style: None,
                extra: None,
                span: Span::new(0, pos),
                append_whitespace: true,
                display_override: None,
                match_indices: None,
            })
            .collect()
    }
}

/// Get the default history file path (~/.claude/venus_history.txt).
pub fn default_history_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".venus").join("venus_history.txt"))
}
