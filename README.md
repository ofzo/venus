# Venus

A terminal-based AI coding assistant built in Rust, powered by the VENUS Claude API. Designed as a feature-complete alternative to Claude Code.

## Features

**Interactive TUI** — ratatui-based terminal interface with streaming markdown rendering, syntax highlighting, vim mode, and interactive pickers.

**40+ Built-in Tools** — Bash, Read, Write, Edit, Glob, Grep, LSP, Agent, MCP, WebFetch, WebSearch, NotebookEdit, Task management, Memory, Cron, Skills, Plan mode, Git worktree, and more.

**Permission System** — Three modes: `default` (ask per tool), `auto` (read-only auto-approved), `bypass` (YOLO). Configurable per-project allow/disallow lists.

**MCP Integration** — Connect external tool servers via Model Context Protocol (stdio/SSE transport).

**LSP Support** — Code intelligence (go-to-definition, find references, hover) via language server protocol.

**Sub-agent System** — Spawn isolated sub-agents for parallel task execution with worktree isolation.

**Memory System** — Persistent cross-session memory with structured types (user, feedback, project, reference).

**Cron Scheduler** — Schedule recurring or one-shot tasks that survive session restarts.

**Plugin System** — Load external tools from plugin directories with manifest-based configuration.

**Configuration** — TOML-based config at `~/.venus/config.toml` (global) or `.venus/config.toml` (project). Provider-based model configuration with aliases.

## Architecture

```
venus/
├── crates/
│   ├── venus-cli/          # Binary: TUI, REPL, CLI parsing, rendering
│   ├── venus-core/         # QueryEngine, message types, tools, LSP, sub-agent, cron
│   ├── venus-tools/        # 40+ tool implementations
│   ├── venus-mcp/          # MCP client (stdio/SSE transport, tool bridge)
│   ├── venus-services/     # VENUS API client, SSE streaming
│   ├── venus-permissions/  # Permission pipeline, rule matching, TUI handler
│   └── venus-utils/        # Config, git, cost tracking, memory, session, helpers
└── tests/
```

## Prerequisites

- Rust (stable, via [rustup](https://rustup.rs))
- [ripgrep](https://github.com/BurntSushi/ripgrep) (`rg`) for the Grep tool
- An VENUS API key (or compatible provider)

## Build

```bash
cargo build --release
```

The binary is at `target/release/venus`.

## Configuration

```bash
# Copy example config
mkdir -p ~/.venus
cp config.example.toml ~/.venus/config.toml

# Edit with your API key
$EDITOR ~/.venus/config.toml
```

See [`config.example.toml`](config.example.toml) for all options including provider setup, model aliases, permission rules, and tool configuration.

## Usage

```bash
# Interactive mode
venus

# Single prompt (non-interactive)
venus -p "list all Rust files in the current directory"

# Specify model
venus -m claude-opus-4-20250514

# Custom working directory
venus -d /path/to/project
```

### Slash Commands

| Command | Description |
|---------|-------------|
| `/help` | Show available commands and keybindings |
| `/exit`, `/quit` | Exit the REPL |
| `/clear` | Clear conversation history |
| `/cost` | Show token usage and cost |
| `/model [name]` | Show or change model |
| `/config`, `/settings` | Open interactive config panel |
| `/diff` | Interactive diff viewer |
| `/compact` | Manually compact conversation context |
| `/history` | Browse conversation history |
| `/commit` | AI-assisted git commit |
| `/review` | AI code review |
| `/memory` | Manage persistent memory |
| `/skills` | Browse and manage skills |
| `/tasks` | View background tasks |
| `/plan` | Enter/exit plan mode |
| `/vim` | Toggle vim mode |
| `/effort` | Adjust reasoning effort level |
| `/copy` | Copy last response to clipboard |
| `/sessions` | Browse session history |
| `/resume` | Resume a previous session |
| `/doctor` | Diagnose configuration issues |
| `/tokens` | Detailed token breakdown |
| `/plugins` | Manage plugins |
| `/context` | Show context window usage |
| `/init` | Initialize CLAUDE.md |
| `/status` | Show system status |
| `/version` | Show version info |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Escape` | Cancel / dismiss |
| `Ctrl+C` | Interrupt / exit |
| `Ctrl+L` | Clear screen |
| `Ctrl+S` | Stash current prompt |
| `Ctrl+Home/End` | Scroll to top/bottom |
| `Tab` | Autocomplete |
| `@` | File path completion |
| `Up/Down` | History navigation |

## License

MIT
