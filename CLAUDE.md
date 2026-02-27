# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Bog

Bog is a Rust-based agent-first codebase annotation system. It uses `.bog` sidecar files that live alongside source code, validated against actual code via tree-sitter. The system enforces module autonomy with gates: owning agents have internal freedom, but cross-module changes go through formal change request protocols.

## Build & Development Commands

Single Rust crate: `bog` (core library + CLI + orchestration).

```bash
cargo build                          # Build
cargo test                           # Run all tests (75 total)
cargo test --lib                     # Run unit tests only (55)
cargo test --test integration        # Run integration tests only (20)
cargo clippy                         # Lint
cargo run -- validate .              # Validate .bog files against source
cargo run -- status .                # Subsystem health report
cargo run -- check .                 # Ownership consistency check
cargo run -- skim .                  # Skimsystem overview
cargo run -- skim . --name code-quality --action clippy  # Run specific integration
cargo run -- stub .                  # Generate annotation stubs
cargo run -- stub . --list           # List existing stubs
cargo run -- orchestrate run "request"              # Full orchestration
cargo run -- orchestrate run "request" --plan-only  # Dry-run (dock plan only)
cargo run -- orchestrate skim code-quality          # Skim lifecycle orchestration
```

Run a single test: `cargo test <test_name>` (e.g., `cargo test parse_repo_annotation`)

## Project Structure

```
bog/
├── Cargo.toml          # Crate manifest
├── bog.toml            # Agent registry, tree-sitter config, health dimensions
├── repo.bog            # Repo-level declarations: subsystems, skimsystems, policies
├── src/                # Library + binary source
│   ├── ast.rs, parser.rs, config.rs, lib.rs    # Core subsystem
│   ├── validator.rs, treesitter.rs, health.rs, stub.rs, integration.rs  # Analysis subsystem
│   ├── cli.rs, main.rs, context.rs             # CLI subsystem
│   └── orchestrate/    # Multi-agent orchestration module
│       ├── mod.rs, orchestrator.rs, dock.rs, agent.rs, skim.rs
│       ├── context.rs, plan.rs, permissions.rs, prompt.rs
│       └── provider.rs, worktree.rs, error.rs
└── tests/              # Integration tests + fixtures
```

## Architecture

### Subsystems

Three subsystems + test-fixtures declared in `repo.bog`:

- **Core** (`core-agent`): Data model, parser, config — `src/{ast,parser,config,lib}.rs`
- **Analysis** (`analysis-agent`): Validation, tree-sitter bridge, health, stubs, integrations — `src/{validator,treesitter,health,stub,integration}.rs`
- **CLI** (`cli-agent`): Command handlers and entry point — `src/{cli,main,context}.rs`
- **Test Fixtures** (`analysis-agent`): `tests/fixtures/src/auth.rs`, `tests/integration.rs`

Three **skimsystems** (cross-cutting quality observers):
- `annotation-quality` (`bog-health-agent`): .bog completeness and hygiene
- `code-quality` (`code-standards-agent`): Pedantic clippy, generates change_requests for subsystem owners
- `tracing` (`observability-agent`): Tracing instrumentation coverage (status: red)

CLI commands: `init`, `validate`, `status`, `check`, `skim`, `stub`, `context`, `orchestrate`

### Orchestration Module (`src/orchestrate/`)

Multi-agent orchestration layer that spawns Claude CLI subprocesses with file-boundary enforcement. Accessed via `bog orchestrate run` and `bog orchestrate skim`.

- **`orchestrator.rs`** — Main `run` loop: dock → plan → delegate → validate → merge/reject with replan. Supports `AllOrNothing` and `Incremental` merge strategies.
- **`dock.rs`** — Dock agent: invokes Claude CLI read-only to produce a `DockPlan` (JSON with tasks assigned to agents). Handles replan with violation feedback.
- **`skim.rs`** — Skim lifecycle: run integration → collect pending change_requests by subsystem → delegate to subsystem agents → validate → merge. `SkimRunResult` and `SubsystemWorkPacket` types.
- **`agent.rs`** — Executes agent tasks in git worktrees, auto-commits, inspects diffs
- **`context.rs`** — Loads `repo.bog` + `bog.toml` into `RepoContext` (queryable agent/subsystem/skimsystem maps, prompt formatting helpers)
- **`permissions.rs`** — Checks diffs against subsystem file globs (subsystem agents) or `.bog`-only rule (skimsystem agents)
- **`worktree.rs`** — Git worktree lifecycle: create, diff inspect, merge, cleanup
- **`provider.rs`** — `Provider` trait + `ClaudeCliProvider` + `MockProvider` (test). Invokes `claude` CLI with `--system-prompt`, `--output-format json`, env-removes `CLAUDECODE` for nesting.
- **`prompt.rs`** — Builds system prompts for dock/subsystem/skimsystem agents from `RepoContext`. Includes replan prompts with violation feedback.
- **`plan.rs`** — `DockPlan`, `AgentTask`, `AgentResult`, `AgentResultStatus` types + topological sort + validation
- **`error.rs`** — `OrchestrateError`, `ProviderError`, `WorktreeError` (all `thiserror`-derived)

**Orchestration flow** (`run`): User prompt → Dock agent (plans, read-only) → Subsystem/Skimsystem agents (execute in isolated git worktrees) → Diff validation → Merge or reject with replan (up to N attempts).

**Skim lifecycle** (`skim`): Run `bog skim` integration → Collect pending `change_request`s grouped by subsystem → Delegate to subsystem agents → Validate → Merge.

**Permission enforcement**: Subsystem agents can only modify files matching their declared file lists in `repo.bog`. Skimsystem agents can only modify `*.bog` files. Violations reject the run.

### Key Data Flow (Core)

1. `parser.rs` uses a PEG grammar (`parser.pest`) to parse `.bog` files into AST types defined in `ast.rs`
2. `treesitter.rs` extracts Rust symbols from source files
3. `validator.rs` cross-references `.bog` annotations against tree-sitter extracted symbols
4. `health.rs` aggregates health dimensions from file-level to subsystem-level to repo-level
5. `integration.rs` runs external tools (e.g., clippy), maps findings to subsystems, generates change_requests in `.bog` files
6. `stub.rs` finds `.rs` files without `.bog` coverage and generates annotation stubs

### Annotation Format

`.bog` files use a bracket-based syntax: `#[annotation_type(args) { body }]`. Three shapes:
- Flat key-value: `#[name(key = value, ...)]`
- Named block: `#[name(identifier) { key = value }]`
- Body block: `#[name { freeform content }]`

Values can be strings, identifiers, booleans, statuses (`green`/`yellow`/`red`), numbers, paths (with `::`), function refs (`fn(name)`), lists, and tuples.

### Configuration

- `bog.toml` — Agent registry (7 agents: core-agent, analysis-agent, cli-agent, orchestrate-agent, code-standards-agent, bog-health-agent, observability-agent), tree-sitter language, health dimensions
- `repo.bog` — Subsystems (core, analysis, cli, orchestrate, test-fixtures), skimsystems (annotation-quality, code-quality, tracing), policies
- `*.rs.bog` — File-level sidecar annotations: ownership, health, function contracts, skim observations, change requests

## Testing

- **Unit tests** live in each module file behind `#[cfg(test)]`
- **Integration tests** in `tests/integration.rs` use `workspace_root()` helper resolving from `CARGO_MANIFEST_DIR`
- Test fixtures live in `tests/fixtures/src/`
- Orchestrate tests use `load_ctx()` / `workspace_root()` helpers resolving from `CARGO_MANIFEST_DIR`
