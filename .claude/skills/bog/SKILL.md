---
name: bog
description: Run bog CLI commands — validate annotations, check health, run skimsystems, generate stubs. Use when the user wants to validate .bog files, check subsystem health, run quality checks, or generate annotation stubs.
allowed-tools: Bash, Read, Grep, Glob
argument-hint: <command> [args]
---

# Bog CLI

Run the bog annotation system CLI. The binary is at `crates/bog/` in the workspace.

## Available Commands

| Command | Usage | Description |
|---------|-------|-------------|
| `validate` | `cargo run -p bog -- validate .` | Validate all .bog files (syntax + tree-sitter cross-reference) |
| `status` | `cargo run -p bog -- status .` | Show health status for all subsystems |
| `check` | `cargo run -p bog -- check .` | Check subsystem/file ownership consistency |
| `skim` | `cargo run -p bog -- skim .` | Show skimsystem health overview |
| `skim --name <name>` | `cargo run -p bog -- skim . --name code-quality` | Run a specific skimsystem's integrations |
| `skim --name <name> --action <action>` | `cargo run -p bog -- skim . --name code-quality --action clippy` | Run a specific integration action |
| `stub` | `cargo run -p bog -- stub .` | Generate stub annotations for unannotated functions |
| `stub --list` | `cargo run -p bog -- stub . --list` | List all current stub annotations |
| `init` | `cargo run -p bog -- init` | Initialize bog in the current directory |

## Instructions

Given user input `$ARGUMENTS`:

1. Parse which command the user wants. If ambiguous, ask.
2. Run the command from the workspace root.
3. Read the output and explain any errors, warnings, or health statuses.
4. If validation fails, read the relevant .bog and .rs files to diagnose and offer fixes.
5. If health is yellow/red, explain which dimensions are degraded and suggest actions.

### Common Workflows

**Full validation check:**
```bash
cargo run -p bog -- validate . && cargo run -p bog -- check . && cargo run -p bog -- status .
```

**Run all quality checks:**
```bash
cargo run -p bog -- skim . --name code-quality --action clippy
cargo run -p bog -- skim . --name annotation-quality
```

**Bootstrap annotations for new files:**
```bash
cargo run -p bog -- stub .
cargo run -p bog -- stub . --list
```
