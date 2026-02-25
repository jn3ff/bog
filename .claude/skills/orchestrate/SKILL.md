---
name: orchestrate
description: Run bog-orchestrate to delegate work to subsystem and skimsystem agents via Claude CLI subprocesses. Use when the user wants to make code changes through the multi-agent orchestration system, plan agent tasks, or run the dock agent.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit
argument-hint: <request>
---

# Bog Orchestrate

Multi-agent orchestration for bog. Spawns Claude CLI subprocesses as subsystem/skimsystem agents with file-boundary enforcement via git worktrees.

## How It Works

1. **Dock agent** (Claude CLI, read-only) analyzes the request and produces a plan
2. **Subsystem agents** execute in isolated git worktrees, can only modify their owned files
3. **Skimsystem agents** execute in worktrees, can only modify .bog files
4. **Orchestrator** validates diffs against ownership boundaries, merges or rejects with replan

## Usage

Given user input `$ARGUMENTS`:

### Dry Run (Plan Only)
Show what the dock agent would plan without executing:
```bash
cargo run -p bog-orchestrate -- --plan-only "$ARGUMENTS"
```

### Full Orchestration
Run the complete dock → plan → delegate → validate → merge loop:
```bash
cargo run -p bog-orchestrate -- "$ARGUMENTS"
```

### With Options
```bash
# More replan attempts on permission violations
cargo run -p bog-orchestrate -- --max-replans 3 "$ARGUMENTS"

# Merge each agent's changes as they succeed (rather than all-or-nothing)
cargo run -p bog-orchestrate -- --merge-strategy incremental "$ARGUMENTS"

# Specify project root if not in current directory
cargo run -p bog-orchestrate -- --path /path/to/project "$ARGUMENTS"
```

## Instructions

1. If the user provides a request, determine if they want a dry run or full execution.
   - Default to `--plan-only` first so the user can review the plan.
   - Ask before running full orchestration (it spawns Claude subprocesses and modifies files).
2. Run the command from the workspace root.
3. Interpret the output:
   - **Plan-only**: Show the dock's plan — which agents, what tasks, file targets.
   - **Full run OK**: Report which agents ran, what files were modified, merge status.
   - **Full run FAIL**: Report permission violations, which agent violated which boundary, and suggest how to fix the plan.
4. If there are violations, explain why (agent tried to modify files outside its subsystem globs).

## Agent Boundaries

These are enforced by the orchestrator via diff inspection after each agent runs:

| Agent Type | Can Modify | Cannot Modify |
|-----------|-----------|---------------|
| Subsystem (`core-agent`) | Files matching their subsystem's `files` globs in `repo.bog` | Any file outside their globs |
| Subsystem (`analysis-agent`) | `crates/bog/src/{treesitter,validator,health,stub,integration}.rs` | Any other .rs file |
| Subsystem (`cli-agent`) | `crates/bog/src/{cli,main}.rs` | Any other .rs file |
| Skimsystem (`quality-agent`) | Any `*.bog` file | Any source file (*.rs, etc.) |

## Current Agents (from bog.toml)

- `core-agent` (subsystem): Owns parsing, AST, and config
- `analysis-agent` (subsystem): Owns tree-sitter bridge, validation, and health reporting
- `cli-agent` (subsystem): Owns CLI interface and entry point
- `quality-agent` (skimsystem): Cross-cutting annotation quality observer
