# Venus

A terminal-based AI coding assistant built in Rust, powered by the Anthropic Claude API.

## Features

- Interactive REPL with streaming responses
- 6 built-in tools: Bash, Read, Write, Edit, Glob, Grep
- Interactive permission system for write operations
- CLAUDE.md instruction loading (global / user / project)
- Git-aware context in system prompts
- Token usage and cost tracking per model
- Slash commands (`/help`, `/clear`, `/cost`, `/model`, `/exit`)

## Architecture

```
venus/
├── crates/
│   ├── venus-cli/          # Binary: REPL, CLI parsing, rendering
│   ├── venus-core/         # QueryEngine, message types, tool trait
│   ├── venus-tools/        # 6 core tool implementations
│   ├── venus-services/     # Anthropic API client, SSE streaming
│   ├── venus-permissions/  # Interactive permission handler
│   └── venus-utils/        # Config, git, cost tracking, helpers
└── tests/
```

## Prerequisites

- Rust 1.75+ (install via [rustup](https://rustup.rs))
- [ripgrep](https://github.com/BurntSushi/ripgrep) (`rg`) for the Grep tool
- An Anthropic API key

## Build

```bash
cargo build --release
```

The binary is at `target/release/venus`.

## Usage

```bash
# Set your API key
export ANTHROPIC_API_KEY=sk-ant-...

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

| Command         | Description                  |
|-----------------|------------------------------|
| `/help`         | Show available commands      |
| `/exit`, `/quit`| Exit the REPL                |
| `/clear`        | Clear conversation history   |
| `/cost`         | Show token usage and cost    |
| `/model [name]` | Show or change model         |

### Environment Variables

| Variable            | Description                |
|---------------------|----------------------------|
| `ANTHROPIC_API_KEY` | API key (required)         |
| `ANTHROPIC_MODEL`   | Default model              |
| `ANTHROPIC_BASE_URL`| Custom API base URL        |
| `RUST_LOG`          | Log level (`debug`, `info`)|

## License

MIT
