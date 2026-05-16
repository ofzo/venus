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
];

/// A REPL input handler backed by reedline.
pub struct InputEditor {
    editor: Reedline,
    prompt: DefaultPrompt,
    vim_mode: bool,
    history_path: Option<PathBuf>,
}

impl InputEditor {
    pub fn new(history_path: Option<PathBuf>) -> Self {
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

        let edit_mode = Box::new(Emacs::new(keybindings));

        let mut editor = Reedline::create()
            .with_completer(completer)
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
            .with_edit_mode(edit_mode);

        if let Some(history) = history {
            editor = editor.with_history(Box::new(history));
        }

        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(">".to_string()),
            DefaultPromptSegment::Empty,
        );

        Self { editor, prompt, vim_mode: false, history_path }
    }

    /// Toggle between Emacs and Vi edit modes.
    pub fn toggle_vim_mode(&mut self) -> bool {
        self.vim_mode = !self.vim_mode;
        // Rebuild the editor with the new edit mode
        *self = InputEditor::new(self.history_path.clone());
        self.vim_mode = !self.vim_mode; // set to desired state
        // If vim was just toggled ON, rebuild again with vim
        if self.vim_mode {
            let completer = Box::new(SlashCompleter);
            let completion_menu = Box::new(
                ColumnarMenu::default()
                    .with_name("completion_menu")
                    .with_columns(4),
            );
            let insert_keybindings = default_vi_insert_keybindings();
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

    /// Read a line of input. Returns None on Ctrl+D (EOF).
    pub fn read_line(&mut self) -> Option<String> {
        match self.editor.read_line(&self.prompt) {
            Ok(Signal::Success(line)) => {
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
    dirs::home_dir().map(|h| h.join(".claude").join("venus_history.txt"))
}
