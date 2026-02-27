---
name: orchestrate
description: Run bog orchestrate to delegate work to subsystem and skimsystem agents via Claude CLI subprocesses. Use when the user wants to make code changes through the multi-agent orchestration system, plan agent tasks, or run the dock agent.
allowed-tools: Bash, Read, Grep, Glob, Write, Edit
argument-hint: <request>
---

# Bog Orchestrate

Multi-agent orchestration for bog. Spawns Claude CLI subprocesses as subsystem/skimsystem agents with file-boundary enforcement via git worktrees.

## How It Works

1. **Dock agent** (Claude CLI, read-only) analyzes the request, reads `repo.bog` and `bog.toml`, and produces a plan — which agents should do what
2. **Subsystem agents** execute in isolated git worktrees, can only modify files matching their subsystem's declared globs
3. **Skimsystem agents** execute in worktrees, can only modify `*.bog` files (they create change_requests, never touch source)
4. **Orchestrator** validates each agent's diff against ownership boundaries, merges successful runs or rejects with replan

Permission enforcement is automatic: after each agent runs, its diff is checked against `repo.bog` declarations. Violations reject the run and trigger replanning.

## Discovery

Before orchestrating, understand the system. The dock agent does this automatically, but you should too:

```
Read repo.bog    → what subsystems and skimsystems exist, who owns what files
Read bog.toml    → what agents are registered, their roles (subsystem vs skimsystem)
bog status .     → current health of all subsystems
bog context . --requests  → pending change_requests that could drive skim lifecycles
```

## Usage

Given user input `$ARGUMENTS`:

### Dry Run (Plan Only)
Show what the dock agent would plan without executing:
```bash
bog orchestrate run --plan-only "$ARGUMENTS"
```

### Full Orchestration
Run the complete dock → plan → delegate → validate → merge loop:
```bash
bog orchestrate run "$ARGUMENTS"
```

### Skim Lifecycle
Run a skimsystem's integration → delegate → resolve → close lifecycle. This is the automated quality loop: run an integration (e.g., clippy), collect the change_requests it generates, delegate fixes to the appropriate subsystem agents.
```bash
bog orchestrate skim <skimsystem-name>
bog orchestrate skim <skimsystem-name> --action <integration>
```

### Options
```bash
# More replan attempts on permission violations (default: 2)
bog orchestrate run --max-replans 3 "$ARGUMENTS"

# Merge each agent's changes as they succeed (default: all-or-nothing)
bog orchestrate run --merge-strategy incremental "$ARGUMENTS"

# Specify project root
bog orchestrate -p /path/to/project run "$ARGUMENTS"
```

## Instructions

1. **Always discover first.** Read `repo.bog` and `bog.toml` to understand what agents exist and what they own before running any orchestration.
2. **Default to plan-only.** Show the dock's plan before executing. Orchestration spawns Claude subprocesses and modifies files — the user should review first.
3. **Interpret results:**
   - **Plan-only**: Show which agents, what tasks, what file targets.
   - **Full run OK**: Report which agents ran, what files were modified, merge status.
   - **Full run FAIL**: Report permission violations — which agent tried to modify which file outside its boundary — and suggest how to fix the plan.
4. **Permission violations** mean an agent tried to modify files outside its declared scope in `repo.bog`. The fix is usually to reassign the task to the correct subsystem agent, or split the task so each agent handles its own files.
5. **Skim lifecycles** are the primary automated quality loop. A skimsystem integration (e.g., clippy) generates change_requests in `.bog` files, then subsystem agents are spawned to fix the underlying issues.

## Agent Boundaries

These are enforced by diff inspection after each agent runs:

| Agent Role | Can Modify | Cannot Modify |
|-----------|-----------|---------------|
| Subsystem agent | Files matching their subsystem's `files` globs in `repo.bog` | Anything outside their globs |
| Skimsystem agent | Any `*.bog` file | Any source file — they observe and create change_requests only |
| Unregistered agent | Nothing | Everything — will be rejected |
