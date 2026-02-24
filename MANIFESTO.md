# bog

## The Problem

When multiple AI agents maintain a codebase, there is no shared protocol for answering basic questions: Who owns this file? What does this function promise? Is this subsystem healthy? Can I change this contract, and who do I ask?

Agents resort to re-reading entire codebases, guessing at boundaries, and making changes that silently break assumptions held by other agents. The codebase has no memory of intent.

## The Design

Bog is a structured annotation layer that lives alongside source code as `.bog` sidecar files. It is not documentation for humans. It is a coordination protocol for agents.

The core principle is **module autonomy with gates**: each subsystem's owning agent has full freedom internally. Cross-module changes go through a formal change request protocol. Hard shell, soft interior — at the organizational level.

Annotations are validated against the actual code via tree-sitter analysis. A `.bog` file that claims a function exists when it doesn't is a validation error, not a suggestion.

## File Layout

```
project/
├── bogbot.toml          # Tooling config: agent registry, tree-sitter settings
├── repo.bog             # Repo-level: subsystem declarations, policies
├── src/
│   ├── auth.rs
│   ├── auth.rs.bog      # Sidecar: ownership, contracts, health, change requests
│   ├── db.rs
│   └── db.rs.bog
```

Every `.bog` file is a sidecar to a source file. `repo.bog` is the root declaration. `bogbot.toml` is plain TOML config.

## Annotation Syntax

Annotations use `#[]` attribute blocks. Three shapes:

```
#[name(key = value, ...)]            — flat key-value
#[name(identifier) { key = value }]  — named block
#[name { content }]                  — body block
```

Line comments with `//`.

## Annotation Reference

### `bogbot.toml`

```toml
[bogbot]
version = "0.1.0"

[agents]
auth-agent = { description = "Owns authentication" }
db-agent = { description = "Owns database layer" }

[tree_sitter]
language = "rust"

[health]
dimensions = ["test_coverage", "staleness", "complexity", "contract_compliance"]
```

### `repo.bog`

#### `#[repo(...)]` — Repository metadata

```
#[repo(
  name = "my-project",
  version = "0.1.0",
  updated = "2026-02-18"
)]
```

| Field | Type | Required |
|-------|------|----------|
| `name` | string | yes |
| `version` | string | yes |
| `updated` | date string | yes |

#### `#[subsystem(name) { ... }]` — Subsystem declaration

```
#[subsystem(authentication) {
  owner = "auth-agent",
  files = ["src/auth/*.rs", "src/auth.rs"],
  status = green,
  description = "JWT-based authentication"
}]
```

| Field | Type | Required |
|-------|------|----------|
| `owner` | string (agent ID) | yes |
| `files` | list of glob patterns | yes |
| `status` | `green` / `yellow` / `red` | yes |
| `description` | string | no |

#### `#[policies { ... }]` — Repo-wide policies

```
#[policies {
  require_contracts = true,
  require_owner = true,
  health_thresholds = {
    red_max_days = 7,
    stale_after_days = 30
  }
}]
```

Open-ended key-value. Agents read these to understand repo norms.

### File Sidecars (`*.rs.bog`)

#### `#[file(...)]` — File ownership

```
#[file(
  owner = "auth-agent",
  subsystem = "authentication",
  updated = "2026-02-18",
  status = green
)]
```

| Field | Type | Required |
|-------|------|----------|
| `owner` | string (agent ID) | yes |
| `subsystem` | string (must match `repo.bog`) | yes |
| `updated` | date string | yes |
| `status` | `green` / `yellow` / `red` | yes |

#### `#[description { ... }]` — Freeform context

```
#[description {
  Handles JWT authentication. Login returns a signed token.
  Refresh tokens rotate every 24 hours.
}]
```

Prose for agents. Design decisions, gotchas, intent.

#### `#[health(...)]` — Health dimensions

```
#[health(
  test_coverage = green,
  staleness = green,
  complexity = yellow,
  security_audit = green
)]
```

Standard dimensions from `bogbot.toml` plus any custom ones. All values are `green`, `yellow`, or `red`.

#### `#[fn(name) { ... }]` — Function annotation

```
#[fn(login) {
  status = green,
  deps = [db::get_user, crypto::verify_hash],
  refs = [routes::post_login],
  contract = {
    in = [(username, String), (password, String)],
    out = "Result<Token, AuthError>",
    invariants = ["never stores plaintext passwords"]
  },
  description = "Authenticates user, returns JWT"
}]
```

| Field | Type | Required |
|-------|------|----------|
| `status` | `green` / `yellow` / `red` | yes |
| `deps` | list of `module::function` paths | no |
| `refs` | list of `module::function` paths | no |
| `contract` | block (see below) | no |
| `description` | string | no |

The function name must exist in the source file. Tree-sitter validates this.

**Contract block:**

| Field | Type | Description |
|-------|------|-------------|
| `in` | list of `(name, Type)` tuples | Parameters |
| `out` | string | Return type |
| `invariants` | list of strings | Guarantees this function maintains |

#### `#[change_requests { ... }]` — Cross-boundary requests

```
#[change_requests {
  #[request(
    id = "cr-001",
    from = "api-agent",
    target = fn(login),
    type = modify_contract,
    status = pending,
    priority = high,
    created = "2026-02-18",
    description = "Need login to accept OAuth tokens"
  )]
}]
```

| Field | Type | Required |
|-------|------|----------|
| `id` | string | yes |
| `from` | string (requesting agent) | yes |
| `target` | reference (`fn(name)`, `file`, etc.) | yes |
| `type` | string (`modify_contract`, `add_fn`, `refactor`, etc.) | yes |
| `status` | string (`pending`, `active`, `closed`, or custom) | yes |
| `priority` | `low` / `medium` / `high` / `critical` | no |
| `created` | date string | yes |
| `description` | string | yes |

This is the gate. An agent that doesn't own a subsystem files a change request instead of making the change. The owning agent reviews and acts on it.

## Validation Rules

1. Every `#[fn(name)]` must correspond to a real function in the source (tree-sitter)
2. Every `#[file(subsystem = "X")]` must match a `#[subsystem(X)]` in `repo.bog`
3. Every `#[file(owner = "Y")]` must match the subsystem's declared owner
4. Every file path must match at least one glob in its subsystem's `files` list
5. Every agent must be registered in `bogbot.toml`

## Status Values

`green` — healthy, no action needed
`yellow` — degraded, attention warranted
`red` — broken, action required

These are assertions, not suggestions. A `red` status is a signal to other agents that this boundary is unstable and cross-boundary changes should wait.

## CLI

```
bogbot init                  # Scaffold bogbot.toml, repo.bog, example sidecar
bogbot validate [path]       # Validate syntax + tree-sitter function matching
bogbot status [path]         # Traffic light health report per subsystem
bogbot check [path]          # Verify ownership consistency
```

## Where This Goes

**v0 (now):** Parse, validate, report. The annotation layer exists and is enforced.

**v1:** Staleness detection — compare annotation dates against git history. Auto-flag files where code changed but annotations didn't.

**v2:** Dependency graph — build a cross-file dependency graph from `deps`/`refs` annotations. Detect when a contract change in one file implies downstream updates.

**v3:** Agent protocol — a machine-readable interface for agents to query bog state, file change requests programmatically, and receive notifications when requests targeting their subsystems are filed.

**v4:** Auto-annotation — given a source file and tree-sitter analysis, generate a skeleton `.bog` sidecar with function stubs, inferred deps, and empty contracts for an agent to fill in.
