# Bog: The Human Guide

Bog is an annotation layer for your codebase. You write `.bog` sidecar files next to your source code that describe what's going on — who owns what, what functions do, what decisions were made, what needs fixing. Then bog validates that those annotations actually match the code.

The whole point: give AI agents (and humans) structured context about your codebase that stays in sync with the actual code.

## Setup

You need a Rust toolchain. Everything runs through cargo:

```bash
cargo run -p bog -- <command> [args]
```

Or build once and use the binary directly:

```bash
cargo build -p bog --release
./target/release/bog <command> [args]
```

## Getting Started

### `bog init`

Scaffolds the config files in your current directory:

```bash
bog init
```

Creates:
- `bog.toml` — agent registry and settings
- `repo.bog` — where you declare subsystems and skimsystems
- `src/main.rs.bog` — example sidecar (if `src/main.rs` exists)

After init, you edit `bog.toml` to register your agents and `repo.bog` to declare your subsystems.

## The Core Loop

The daily workflow is: write code, keep `.bog` files in sync, validate.

### `bog validate .`

The main sanity check. Validates every `.bog` file in your project:

- Syntax — are the annotations well-formed?
- Cross-reference — do `#[fn(name)]` annotations match real functions in the source? (uses tree-sitter)
- Ownership — do file owners and subsystem memberships line up?
- Skim targets — do `#[skim]` observations reference real skimsystems and functions?

```bash
bog validate .
```

```
Validating .bog files...
  Files checked: 28
  All checks passed.
```

If something's wrong, it tells you exactly what:

```
  error: Function 'old_name' in parser.rs.bog not found in parser.rs
  FAIL: 1 error(s) found.
```

**Run this after every code change.** If you renamed a function, added a file, or moved things around — validate catches the drift.

### `bog check .`

A focused subset of validate that only checks ownership consistency:

- Every file annotation claims a subsystem — does that subsystem exist in `repo.bog`?
- Every file claims an owner — is that agent registered in `bog.toml`?
- Are agent roles correct? (subsystem agents own subsystems, skimsystem agents own skimsystems)

```bash
bog check .
```

Useful when you're restructuring subsystem boundaries or adding new agents.

## Understanding Your Project

### `bog status .`

The dashboard. Shows health for every subsystem and skimsystem at a glance:

```bash
bog status .
```

```
Project: bog
  Overall: ● (green)

  ● core (owner: core-agent)
    Files: 4
    Functions: 42 total (42 green, 0 yellow, 0 red)
    test_coverage: ●
    staleness: ●

  ...

  Skimsystems:

  ● annotation-quality (owner: bog-health-agent)
    Observations: 16 total (11 green, 5 yellow, 0 red)

  ● tracing (owner: observability-agent)
    Observations: 27 total (6 green, 0 yellow, 21 red)
```

The traffic lights: green = healthy, yellow = needs attention, red = degraded. Health rolls up worst-wins — any red dimension makes the whole subsystem red.

### `bog context .`

The deep dive. Shows everything bog knows about your annotations — health, function contracts, pickled entries, change requests, skim observations. This is what agents use to load context before working.

```bash
# Everything for the whole repo
bog context .

# Everything for a specific agent's files
bog context . --agent core-agent

# Everything for a specific subsystem
bog context . --subsystem analysis

# Just the pickled entries (agent notes/decisions)
bog context . --agent core-agent --pickled

# Just pending change requests
bog context . --requests

# Just function contracts
bog context . --contracts

# Just skim observations
bog context . --skims

# Just health dimensions
bog context . --health

# Filter pickled by kind
bog context . --pickled --kind decision

# Filter pickled by tag
bog context . --pickled --tag architecture

# JSON output (for piping to other tools)
bog context . --agent core-agent --format json
```

The text output groups everything by subsystem and file:

```
═══ core (owner: core-agent, ●)

  parser.rs
    [health] test_coverage: ●, staleness: ●
    [pickled] (2)
      pickle-001 · decision · [architecture] · 2026-02-24
        "Chose PEG grammar over hand-rolled parser..."
    [requests] (3 pending)
      code-quality-clippy-abc · code-standards-agent → fn(parse_bog) · pending
        "clippy::missing_errors_doc..."
    [fn contracts] (12)
      parse_bog — ●
        in: (input: str) → Result<BogFile, ParseError>
    [skims] (2)
      ● tracing  "No tracing instrumentation..."
      ● code-quality  "clippy: 3 warning(s)"
```

## Quality Observability

### `bog skim .`

Shows the status of your skimsystems — cross-cutting quality observers that watch everything. Each skimsystem has principles (what it cares about) and optionally integrations (tools it can run).

```bash
# Overview of all skimsystems
bog skim .

# Verbose — show individual observations per file
bog skim . --verbose

# Show a specific skimsystem
bog skim . --name code-quality

# Run an integration (e.g., clippy) and write results to .bog files
bog skim . --name code-quality --action clippy
```

The `--action clippy` command is the interesting one. It:
1. Runs `cargo clippy --message-format=json -- -W clippy::pedantic`
2. Parses every warning
3. Maps each warning to the file and function it applies to
4. Writes `#[skim(code-quality)]` observations and `#[change_requests]` into the relevant `.bog` files
5. Prints a summary

After running it, each `.bog` file that had clippy warnings will have specific change requests like:

```
#[change_requests {
  #[request(
    id = "code-quality-clippy-abc123",
    from = "code-standards-agent",
    target = fn(parse_bog),
    type = lint_warning,
    status = pending,
    description = "clippy::missing_errors_doc: docs for function returning Result missing # Errors section"
  )]
}]
```

These change requests are the work queue for subsystem agents. When an agent fixes the issue, it changes `status = pending` to `status = resolved`.

## Bootstrapping Annotations

### `bog stub .`

When you add new source files or functions, they won't have `.bog` annotations yet. Stub generates them:

```bash
# Generate stubs for all unannotated functions
bog stub .

# See what stubs already exist
bog stub . --list
```

Stubs are marked with `stub = true` so you know they need to be filled in:

```
#[fn(my_new_function) {
  status = yellow,
  stub = true,
  deps = [some_function_it_calls],
  description = "TODO"
}]
```

The stub generator uses tree-sitter to find functions, detects which functions they call, and writes the skeleton. You then fill in the description, contract, and correct the status.

## Multi-Agent Orchestration

The `bog-orchestrate` binary handles the full agent loop. This spawns actual Claude CLI subprocesses in isolated git worktrees with file-boundary enforcement.

### `bog-orchestrate run "your request"`

Free-form orchestration. A dock agent reads your repo context and creates a plan, then subsystem agents execute their tasks in parallel:

```bash
# Full orchestration
cargo run -p bog-orchestrate -- run "Add error handling to the parser"

# Dry run — just see the plan, don't execute
cargo run -p bog-orchestrate -- run "Add error handling to the parser" --plan-only
```

Flow: your request -> dock agent plans -> subsystem agents execute in git worktrees -> permission check (did they stay in their lane?) -> merge or reject with replan.

### `bog-orchestrate skim code-quality`

Runs the full skimsystem lifecycle automatically:

```bash
cargo run -p bog-orchestrate -- skim code-quality
```

Flow: run the integration (clippy) -> collect pending change requests grouped by subsystem -> delegate to subsystem agents -> validate their changes -> merge.

This is the automated version of: run clippy, see what's broken, assign fixes to the right agents, let them fix it, merge.

## The .bog File Format

Every `.rs` file can have a `.rs.bog` sidecar. For example, `src/parser.rs` has `src/parser.rs.bog`.

### File-level annotations

```
#[file(
  owner = "core-agent",
  subsystem = "core",
  updated = "2026-02-26",
  status = green
)]

#[description {
  What this file does, in plain English.
}]

#[health(
  test_coverage = green,
  staleness = green,
  complexity = yellow,
  contract_compliance = green
)]
```

### Function contracts

```
#[fn(parse_bog) {
  status = green,
  deps = [parse_annotation, extract_kv_map],
  contract = {
    in = [(input, str)],
    out = "Result<BogFile, ParseError>",
    invariants = ["never panics on malformed input"]
  },
  description = "Parses a .bog file into an AST"
}]
```

### Skim observations

Written by skimsystem agents (or integrations like clippy):

```
#[skim(tracing) {
  status = red,
  notes = "No tracing instrumentation. Needs INFO at startup, WARN on errors."
}]

#[skim(code-quality) {
  status = green,
  notes = "No clippy warnings"
}]
```

### Change requests

Cross-module work items. A skimsystem agent spots a problem in someone else's code and files a request:

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

### Pickled entries (agent memory)

How agents build up context over time. Decisions, reversals, observations, domain knowledge:

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

#[pickled(agent = "core-agent", updated = "2026-02-26") {
  id = "pickle-003",
  kind = context,
  tags = [domain],
  content = "The contract parsing uses nested blocks — watch out for the kv_list extraction order."
}]
```

**Kinds**: `decision`, `reversal`, `context`, `observation`, `rationale`

**Tags**: `architecture`, `performance`, `reliability`, `security`, `testing`, `domain`, `debt`, `tooling`

Reversals can reference the entry they override with `supersedes`.

### Repo-level declarations (repo.bog)

```
#[repo(name = "my-project", version = "0.1.0", updated = "2026-02-26")]

#[subsystem(core) {
  owner = "core-agent",
  files = ["src/ast.rs", "src/parser.rs", "src/config.rs"],
  status = green,
  description = "Data model and parser"
}]

#[skimsystem(code-quality) {
  owner = "code-standards-agent",
  targets = all,
  status = green,
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

#[policies {
  require_contracts = true,
  require_owner = true
}]
```

## Typical Workflow

```bash
# 1. Check the lay of the land
bog status .

# 2. See what needs attention
bog context . --requests          # pending work
bog context . --pickled           # what agents know

# 3. Run quality checks
bog skim . --name code-quality --action clippy

# 4. See what clippy found
bog context . --requests

# 5. After making changes, validate
bog validate .

# 6. Generate stubs for any new functions
bog stub .

# 7. Fill in the stubs, then validate again
bog validate .
```

## Config Reference

### bog.toml

```toml
[bog]
version = "0.1.0"

[agents]
core-agent = { role = "subsystem", description = "Owns the data model and parser" }
code-standards-agent = { role = "skimsystem", description = "Pedantic clippy enforcement" }

[tree_sitter]
language = "rust"

[health]
dimensions = ["test_coverage", "staleness", "complexity", "contract_compliance"]
```

Agents have two roles: `subsystem` (owns files, can modify source) and `skimsystem` (observes everything, can only modify `.bog` files).
