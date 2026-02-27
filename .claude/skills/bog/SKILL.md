---
name: bog
description: Run bog CLI commands and read .bog annotation files to understand the codebase. Use when the user wants to validate .bog files, check subsystem health, run quality checks, generate stubs, understand what's happening in a part of the system, or inspect agent context/decisions.
allowed-tools: Bash, Read, Grep, Glob
argument-hint: <command or question> [args]
---

# Bog

Bog is an agent-first codebase annotation system. `.bog` sidecar files live alongside source code, validated against actual code via tree-sitter. The system enforces module autonomy: owning agents have internal freedom, cross-module changes go through change_requests.

## Discovery — Start Here

Every bog project has two config files at the root. **Read these first** to understand the system:

1. **`repo.bog`** — The declaration of truth. Contains subsystem definitions (who owns what files), skimsystem definitions (cross-cutting quality observers), and policies. This tells you the shape of the codebase.
2. **`bog.toml`** — Agent registry and config. Lists all registered agents, their roles (subsystem vs skimsystem), tree-sitter language, and health dimensions.

From these two files you can answer: what subsystems exist, who owns them, what files belong to each, what quality observers are active, and what agents are registered.

Then discover the annotation layer:
```bash
# Find all .bog sidecar files
Glob **/*.bog

# See what a specific file's annotations say
Read src/some_module.rs.bog
```

## CLI Commands

If `bog` is installed, use it directly. Otherwise use `cargo run --`.

| Command | What it does |
|---------|-------------|
| `bog validate .` | Validate all .bog files: syntax, tree-sitter cross-ref, ownership consistency |
| `bog status .` | Subsystem and skimsystem health report with traffic-light rollups |
| `bog check .` | Ownership consistency checks (subsystem membership, agent registration) |
| `bog skim .` | Skimsystem health overview with observation counts |
| `bog skim . --name <name> --action <action>` | Run a specific integration and write results to .bog files |
| `bog context .` | Show all annotation context (health, contracts, requests, pickled, skims) |
| `bog context . --pickled` | Show only pickled annotations (agent decisions/context) |
| `bog context . --requests` | Show only pending change_requests |
| `bog context . --agent <name>` | Scope context to a specific agent's subsystems |
| `bog context . --subsystem <name>` | Scope context to a specific subsystem |
| `bog stub .` | Generate annotation stubs for unannotated functions |
| `bog stub . --list` | List existing stub annotations that need completion |
| `bog init` | Scaffold bog.toml, repo.bog, and example sidecar for a new project |

## Reading .bog Files

Every source file can have a sidecar (e.g., `src/parser.rs` → `src/parser.rs.bog`). These are the annotation layer — what agents know about the code.

**Before modifying code, read the relevant .bog sidecars** to understand ownership, contracts, pending work, and prior decisions.

### Annotation Reference

**Repo-level** (in `repo.bog`):

| Annotation | Purpose |
|-----------|---------|
| `#[repo(name, version, updated)]` | Repo metadata |
| `#[description { text }]` | What the repo does |
| `#[subsystem(name) { owner, files, status, description }]` | Declares a subsystem with file globs and owner |
| `#[skimsystem(name) { owner, targets, status, principles, integrations }]` | Declares a cross-cutting quality observer |
| `#[policies { key = value }]` | Governance flags (require_contracts, health_thresholds) |

**File-level** (in `*.bog` sidecars):

| Annotation | Purpose |
|-----------|---------|
| `#[file(owner, subsystem, updated, status)]` | File ownership and subsystem membership |
| `#[description { text }]` | What this module does |
| `#[health(dimension = status, ...)]` | Traffic-light health dimensions |
| `#[fn(name) { status, deps, contract, description }]` | Function contract with inputs, output, invariants, deps |
| `#[skim(skimsystem) { status, notes, target }]` | Quality observation from a skimsystem |
| `#[change_requests { #[request(...)] ... }]` | Pending cross-module work items |
| `#[pickled(agent, updated) { id, kind, tags, content, supersedes? }]` | Agent context, decisions, accumulated knowledge |

### Pickled Annotations — Agent Memory

Pickled entries are how agents marinate context over time. They store decisions, reversals, observations, and domain knowledge so agents always have the full picture.

**Kinds**: `decision`, `reversal` (supersedes a prior decision), `context`, `observation`, `rationale`

**Tags**: `architecture`, `performance`, `reliability`, `security`, `testing`, `domain`, `debt`, `tooling`

### Status Traffic Lights

`green` = healthy, `yellow` = needs attention, `red` = degraded. Health rolls up worst-wins: any red dimension makes the subsystem red.

### The Grammar

All annotations share one shape: `#[ident(parens?) { body? }]`. Patterns:
- **Flat kv**: `#[name(key = value, ...)]`
- **Named block**: `#[name(identifier) { kv pairs }]`
- **Body block**: `#[name { freeform text or nested annotations }]`
- **Hybrid**: `#[name(kv pairs) { kv pairs }]`

Values: strings (`"quoted"`), identifiers (`bare`), booleans, statuses (`green`/`yellow`/`red`), numbers, paths (`mod::func`), function refs (`fn(name)`), lists (`[a, b]`), tuples, nested blocks.

## Instructions

Given user input `$ARGUMENTS`:

1. **First time in a bog project?** Read `repo.bog` and `bog.toml` to understand the system's shape — subsystems, agents, skimsystems, policies.
2. **If it's a CLI command** — run it, read output, explain errors/warnings/health.
3. **If it's a question about the codebase** — find and read the relevant `.bog` sidecars. Use `Glob **/*.bog` to discover them and `Grep` to search across annotations.
4. **If validation fails** — read both the `.bog` and source files to diagnose the mismatch. Common issues: function renamed/deleted but .bog not updated, file moved but subsystem globs not updated.
5. **If health is yellow/red** — explain which dimensions are degraded and suggest actions.
6. **If there are pending change_requests** — summarize them as a work queue for the owning agent.
7. **If there are pickled entries** — surface relevant decisions and context before suggesting changes. Check for reversals that supersede earlier decisions.
8. **If the user asks what an agent knows** — use `bog context . --agent <name>` or read all .bog files in that agent's subsystems and summarize.

### Workflows

**Understand a new bog project:**
```
Read repo.bog → Read bog.toml → bog status . → bog context .
```

**Full validation:**
```bash
bog validate . && bog check . && bog status .
```

**Explore a subsystem's state:**
Read `repo.bog` for the subsystem declaration, then glob for its `.bog` sidecars. Look for pickled entries, change_requests, and skim observations.

**Understand decisions made about a module:**
```
Grep "pickled" in *.bog files, read the matching entries, follow supersedes chains.
```
