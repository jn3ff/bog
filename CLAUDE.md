# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Bog

Bog is a Rust-based agent-first codebase annotation system. It uses `.bog` sidecar files that live alongside source code, validated against actual code via tree-sitter. The system enforces module autonomy with gates: owning agents have internal freedom, but cross-module changes go through formal change request protocols.

## Build & Development Commands

This is a Cargo workspace with two crates: `bog` (core library + CLI) and `bog-orchestrate` (multi-agent orchestration).

```bash
cargo build --workspace              # Build everything
cargo test --workspace               # Run all tests (63 total)
cargo test -p bog --lib              # Run bog unit tests only
cargo test -p bog --test integration # Run bog integration tests only
cargo test -p bog-orchestrate        # Run orchestrate tests only
cargo clippy --workspace             # Lint
cargo run -p bog -- validate .       # Validate all .bog files against source
cargo run -p bog -- status .         # Show subsystem health report
cargo run -p bog -- stub .           # Auto-generate annotation stubs
cargo run -p bog-orchestrate -- --plan-only "request" # Dry-run orchestration
```

Run a single test: `cargo test <test_name>` (e.g., `cargo test parse_repo_annotation`)

## Workspace Structure

```
bog/
├── Cargo.toml          # Workspace root
├── bog.toml            # Agent registry, tree-sitter config, health dimensions
├── repo.bog            # Repo-level declarations: subsystems, skimsystems, policies
├── crates/
│   ├── bog/            # Core library + CLI binary
│   │   ├── src/        # ast, parser, config, validator, health, integration, stub, treesitter, cli
│   │   └── tests/      # Integration tests + fixtures
│   └── bog-orchestrate/ # Multi-agent orchestration binary
│       └── src/        # orchestrator, dock, agent, provider, prompt, context, permissions, worktree, plan
```

## Architecture

### Core Crate (`crates/bog/`)

Three subsystems declared in `repo.bog`, each owned by an agent:

- **Core** (`core-agent`): Data model, parser, config — `crates/bog/src/{ast,parser,config,lib}.rs`
- **Analysis** (`analysis-agent`): Validation, tree-sitter bridge, health, stubs, integrations — `crates/bog/src/{validator,treesitter,health,stub,integration}.rs`
- **CLI** (`cli-agent`): Command handlers and entry point — `crates/bog/src/{cli,main}.rs`

Two **skimsystems** (cross-cutting quality observers): `annotation-quality` and `code-quality`, both owned by `quality-agent`.

### Orchestration Crate (`crates/bog-orchestrate/`)

Multi-agent orchestration layer that spawns Claude CLI subprocesses with file-boundary enforcement:

- **`context.rs`** — Loads `repo.bog` + `bog.toml` into `RepoContext` (queryable agent/subsystem/skimsystem maps)
- **`dock.rs`** — Dock agent: invokes Claude CLI read-only to produce a `DockPlan` (JSON with tasks assigned to agents)
- **`agent.rs`** — Executes agent tasks in git worktrees, auto-commits, inspects diffs
- **`orchestrator.rs`** — Main loop: dock → plan → delegate → validate → merge/reject with replan
- **`permissions.rs`** — Checks diffs against subsystem file globs (subsystem agents) or `.bog`-only rule (skimsystem agents)
- **`worktree.rs`** — Git worktree lifecycle: create, diff inspect, merge, cleanup
- **`provider.rs`** — `Provider` trait + `ClaudeCliProvider` (designed for future provider toggleability)
- **`prompt.rs`** — Builds system prompts for dock/subsystem/skimsystem agents from `RepoContext`
- **`plan.rs`** — `DockPlan`, `AgentTask`, `AgentResult` types + topological sort + validation

**Orchestration flow**: User prompt → Dock agent (plans, read-only) → Subsystem/Skimsystem agents (execute in isolated git worktrees) → Diff validation against ownership boundaries → Merge or reject with replan.

**Permission enforcement**: Subsystem agents can only modify files matching their glob patterns in `repo.bog`. Skimsystem agents can only modify `*.bog` files. Violations reject the entire run and surface to the dock for replanning.

### Key Data Flow (Core)

1. `parser.rs` uses a PEG grammar (`parser.pest`) to parse `.bog` files into AST types defined in `ast.rs`
2. `treesitter.rs` extracts Rust symbols from source files
3. `validator.rs` cross-references `.bog` annotations against tree-sitter extracted symbols
4. `health.rs` aggregates health dimensions from file-level to subsystem-level to repo-level
5. `integration.rs` runs external tools (e.g., clippy), maps findings to subsystems, generates change_requests
6. `stub.rs` finds `.rs` files without `.bog` coverage and generates annotation stubs

### Annotation Format

`.bog` files use a bracket-based syntax: `#[annotation_type(args) { body }]`. Three shapes:
- Flat key-value: `#[name(key = value, ...)]`
- Named block: `#[name(identifier) { key = value }]`
- Body block: `#[name { freeform content }]`

Values can be strings, identifiers, booleans, statuses (`green`/`yellow`/`red`), numbers, paths (with `::`), function refs (`fn(name)`), lists, and tuples.

### Configuration

- `bog.toml` — Agent registry (role + description), tree-sitter language setting, health dimension definitions
- `repo.bog` — Repo-level declarations: subsystems (with file globs and owners), skimsystems, policies
- `*.rs.bog` — File-level sidecar annotations: ownership, health, function contracts, skim observations, change requests

## Testing

- **Unit tests** live in each module file behind `#[cfg(test)]`
- **Integration tests** in `crates/bog/tests/integration.rs` use `workspace_root()` helper to resolve paths from `CARGO_MANIFEST_DIR`
- Test fixtures live in `crates/bog/tests/fixtures/src/`
