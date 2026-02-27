# Bog

*In a bog you are pickled, in a swamp you would decompose.*

Bog is an agent-first codebase annotation system. You write `.bog` sidecar files next to your source code that describe what's going on — who owns what, what functions do, what decisions were made, what needs fixing. Then bog validates those annotations against the actual code via tree-sitter.

The whole point: give AI agents (and humans) structured context about a codebase that stays in sync with the real code. Agents get scoped ownership, formal change request protocols, and a persistent memory layer (pickled entries) — all validated against source.

## Install

Requires a Rust toolchain (edition 2024).

```bash
cargo install --path .
```

Or build and run directly:

```bash
cargo run -- <command> [args]
```

For multi-agent orchestration, you also need the [Claude CLI](https://docs.anthropic.com/en/docs/claude-code) on your PATH.

## Quick Start

```bash
# Initialize bog in an existing project
bog init

# Validate .bog files against source code
bog validate .

# See subsystem health at a glance
bog status .

# Run clippy and write findings to .bog files as change requests
bog skim . --name code-quality --action clippy

# Orchestrate agents to fix those findings
bog orchestrate skim code-quality
```

## How It Works

### Sidecar Files

Every source file can have a `.bog` sidecar. `src/parser.rs` gets `src/parser.rs.bog`:

```
#[file(
  owner = "core-agent",
  subsystem = "core",
  updated = "2026-02-26",
  status = green
)]

#[description {
  Pest-based parser for .bog files. Transforms PEG parse tree into AST types.
}]

#[health(
  test_coverage = green,
  staleness = green,
  complexity = yellow,
  contract_compliance = green
)]

#[fn(parse_bog) {
  status = green,
  contract = {
    in = [(input, str)],
    out = "Result<BogFile, ParseError>",
    invariants = ["returns all annotations in source order"]
  },
  description = "Public entry point: parse .bog text into a BogFile AST"
}]
```

Bog validates these against the actual code — if you rename `parse_bog`, `bog validate` catches the drift.

### Subsystems and Ownership

Your codebase is divided into **subsystems** declared in `repo.bog`. Each subsystem has an owning agent and a list of files:

```
#[subsystem(core) {
  owner = "core-agent",
  files = ["src/ast.rs", "src/parser.rs", "src/config.rs", "src/lib.rs"],
  status = green,
  description = "Data model, .bog parser (pest), and config loading"
}]
```

Agents have internal freedom within their subsystem. Cross-module changes go through **change requests** — formal work items filed in `.bog` files.

### Skimsystems

Cross-cutting quality observers that watch everything but can only write `.bog` files (never source). They file change requests for subsystem owners to act on.

```
#[skimsystem(code-quality) {
  owner = "code-standards-agent",
  targets = all,
  integrations = {
    clippy = {
      command = "cargo clippy --message-format=json -- -W clippy::pedantic",
      format = cargo_diagnostic
    }
  },
  principles = [
    "No pedantic clippy warnings should persist unaddressed"
  ]
}]
```

Running `bog skim . --name code-quality --action clippy` executes the integration, parses every warning, maps each to the owning file/function, and writes `#[change_requests]` into the relevant `.bog` files.

### Change Requests

The work queue. A skimsystem agent spots a problem and files a request. The subsystem owner fixes it and marks it resolved:

```
#[change_requests {
  #[request(
    id = "code-quality-clippy-abc123",
    from = "code-standards-agent",
    target = fn(parse_bog),
    type = lint_warning,
    status = pending,
    created = "2026-02-25",
    description = "clippy::missing_errors_doc: docs for function returning Result missing # Errors section"
  )]
}]
```

### Pickled Entries (Agent Memory)

Persistent notes that agents build up over time — decisions, reversals, domain knowledge:

```
#[pickled(agent = "core-agent", updated = "2026-02-26") {
  id = "pickle-001",
  kind = decision,
  tags = [architecture],
  content = "Chose PEG grammar over hand-rolled parser for maintainability."
}]

#[pickled(agent = "core-agent", updated = "2026-02-26") {
  id = "pickle-002",
  kind = reversal,
  supersedes = "pickle-001",
  tags = [architecture],
  content = "Reverting nested grammar experiment. Error messages became useless."
}]
```

Kinds: `decision`, `reversal`, `context`, `observation`, `rationale`. Tags: `architecture`, `performance`, `reliability`, `security`, `testing`, `domain`, `debt`, `tooling`.

## CLI Reference

| Command | Description |
|---------|-------------|
| `bog init` | Scaffold `bog.toml`, `repo.bog`, and an example sidecar |
| `bog validate .` | Validate `.bog` syntax + tree-sitter cross-references |
| `bog status .` | Subsystem and skimsystem health dashboard |
| `bog check .` | Ownership consistency check |
| `bog skim .` | Skimsystem overview (add `--name X --action Y` to run integrations) |
| `bog context .` | Show annotation context (scoped by `--agent`, `--subsystem`, or section filters) |
| `bog stub .` | Generate annotation stubs for unannotated functions |
| `bog orchestrate run "request"` | Multi-agent orchestration: dock plans, agents execute, merge |
| `bog orchestrate skim code-quality` | Full skimsystem lifecycle: integrate, delegate, resolve |

### Context Filters

```bash
bog context . --agent core-agent          # Scoped to an agent's files
bog context . --subsystem analysis        # Scoped to a subsystem
bog context . --requests                  # Just pending change requests
bog context . --pickled                   # Just pickled entries
bog context . --contracts                 # Just function contracts
bog context . --skims                     # Just skim observations
bog context . --health                    # Just health dimensions
bog context . --pickled --kind decision   # Filter by kind
bog context . --pickled --tag architecture # Filter by tag
bog context . --format json              # JSON output
```

## Multi-Agent Orchestration

Bog includes a multi-agent orchestration layer that spawns Claude CLI subprocesses in isolated git worktrees with file-boundary enforcement. Each agent can only modify files within its declared scope — violations reject the entire run.

### `bog orchestrate run`

Free-form orchestration. A dock agent reads your repo context, creates a plan, then subsystem agents execute their tasks:

```bash
bog orchestrate run "Add error handling to the parser"
bog orchestrate run "Add error handling to the parser" --plan-only  # Dry run
```

Flow: request → dock agent plans (read-only) → subsystem agents execute in git worktrees → permission check → merge or reject with replan.

### `bog orchestrate skim`

Automated skimsystem lifecycle:

```bash
bog orchestrate skim code-quality
bog orchestrate skim --action clippy code-quality
```

Flow: run integration (clippy) → collect pending change requests by subsystem → delegate to subsystem agents → validate → merge.

Real-time progress streaming shows what each agent is doing:

```
[skim] Phase 3: Delegating to subsystem agents...
[skim]   Spawning analysis-agent for subsystem 'analysis'...
  [analysis-agent] turn 1 ▸ Read src/health.rs
  [analysis-agent] turn 2 ▸ Edit src/health.rs, Edit src/health.rs.bog
  [analysis-agent] turn 3 ▸ Read src/validator.rs
  [analysis-agent] ✓ done — 120s, 8 turns, $0.15
```

### Permission Enforcement

- **Subsystem agents** can only modify files matching their subsystem's declared file list
- **Skimsystem agents** can only modify `*.bog` files
- Violations reject the run and trigger replanning (up to N attempts)

## Configuration

### bog.toml

Agent registry, tree-sitter language, and health dimensions:

```toml
[bog]
version = "0.1.0"

[tree_sitter]
language = "rust"

[health]
dimensions = ["test_coverage", "staleness", "complexity", "contract_compliance"]
```

Agents are declared in `repo.bog` as subsystem or skimsystem owners. Two roles: **subsystem** agents own files and can modify source; **skimsystem** agents observe everything and can only modify `.bog` files.

### repo.bog

Top-level declarations: subsystems, skimsystems, policies.

```
#[repo(name = "my-project", version = "0.1.0", updated = "2026-02-26")]

#[subsystem(core) {
  owner = "core-agent",
  files = ["src/ast.rs", "src/parser.rs"],
  status = green,
  description = "Data model and parser"
}]

#[skimsystem(code-quality) {
  owner = "code-standards-agent",
  targets = all,
  status = green,
  principles = ["No clippy warnings should persist"]
}]

#[policies {
  require_contracts = true,
  require_owner = true,
  health_thresholds = { red_max_days = 7, stale_after_days = 30 }
}]
```

## Project Structure

```
bog/
├── Cargo.toml              # Single crate manifest
├── bog.toml                # Agent registry + tree-sitter config
├── repo.bog                # Subsystems, skimsystems, policies
├── src/
│   ├── ast.rs              # Data model (annotation types)
│   ├── parser.rs           # PEG parser (.bog → AST)
│   ├── parser.pest         # PEG grammar
│   ├── config.rs           # bog.toml loading
│   ├── lib.rs              # Library root
│   ├── validator.rs        # Cross-reference validation
│   ├── treesitter.rs       # Tree-sitter symbol extraction
│   ├── health.rs           # Health aggregation
│   ├── stub.rs             # Annotation stub generation
│   ├── integration.rs      # External tool integrations (clippy)
│   ├── cli.rs              # CLI command handlers
│   ├── main.rs             # Entry point
│   ├── context.rs          # Context query + formatting
│   └── orchestrate/        # Multi-agent orchestration
│       ├── orchestrator.rs # Run loop: dock → delegate → merge
│       ├── dock.rs         # Dock agent (planning)
│       ├── agent.rs        # Agent task execution
│       ├── skim.rs         # Skim lifecycle
│       ├── provider.rs     # Claude CLI invocation + stream parsing
│       ├── context.rs      # RepoContext loading
│       ├── prompt.rs       # System prompt generation
│       ├── permissions.rs  # File boundary enforcement
│       ├── worktree.rs     # Git worktree lifecycle
│       ├── plan.rs         # Plan types + topological sort
│       └── error.rs        # Error types
├── tests/
│   ├── integration.rs      # 21 integration tests
│   └── fixtures/           # Test fixture files
└── src/*.rs.bog            # Sidecar annotations for every source file
```

## Development

```bash
cargo build                    # Build
cargo test                     # Run all 84 tests
cargo test --lib               # Unit tests only (63)
cargo test --test integration  # Integration tests only (21)
cargo clippy                   # Lint
```

Bog dogfoods itself — every source file has a `.bog` sidecar, validated by its own integration tests.
