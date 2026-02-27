use crate::ast::{Annotation, BogFile, Status, Value};

use super::context::RepoContext;
use super::plan::AgentTask;

// ---------------------------------------------------------------------------
// Dock agent prompts
// ---------------------------------------------------------------------------

/// Build the system prompt for the dock agent.
pub fn build_dock_system_prompt(ctx: &RepoContext) -> String {
    let mut sections = vec![
        "You are the dock agent for the bog orchestration system.\n\n\
         Your role is to analyze user requests and produce a structured execution plan \
         that delegates work to registered agents. You are READ-ONLY — do not modify any files."
            .to_string(),
    ];

    // Raw repo.bog for full context
    sections.push(format!(
        "## Repository Declarations (repo.bog)\n\n{}",
        ctx.repo_bog_raw
    ));

    // Agent registry
    sections.push(format!(
        "## Agent Registry (from repo.bog)\n\n{}",
        ctx.format_agent_registry()
    ));

    // Rich subsystem summary with descriptions
    sections.push(format!(
        "## Subsystem Ownership\n\n{}",
        render_subsystem_summary_rich(ctx)
    ));

    // Rich skimsystem summary with descriptions
    sections.push(format!(
        "## Skimsystem Coverage\n\n{}",
        render_skimsystem_summary_rich(ctx)
    ));

    // Rules
    sections.push(
        "## Rules\n\n\
         1. Subsystem agents can ONLY modify files matching their subsystem's glob patterns.\n\
         2. Skimsystem agents can ONLY modify *.bog files (never source files). They create change_requests.\n\
         3. Each task must specify a registered agent (an owner declared in repo.bog).\n\
         4. Tasks may depend on other tasks using depends_on with task indices.\n\
         5. Instructions should be specific and actionable.\n\
         6. focus_files should list the specific files the agent should work on."
            .to_string(),
    );

    // Output format
    sections.push(
        "## Output Format\n\n\
         Respond with ONLY a JSON object matching this schema (no markdown, no explanation):\n\
         {\n  \"summary\": \"string — what you plan to do\",\n  \"tasks\": [\n    {\n      \
         \"agent\": \"string — agent name from repo.bog\",\n      \
         \"instruction\": \"string — specific instruction for this agent\",\n      \
         \"focus_files\": [\"string — file paths to focus on\"],\n      \
         \"depends_on\": [0],\n      \
         \"model\": \"string (optional) — model override, e.g. claude-sonnet-4-6 or o4-mini\"\n    \
         }\n  ]\n}"
            .to_string(),
    );

    sections.join("\n\n")
}

/// Build a replan prompt that includes violation feedback from a previous attempt.
pub fn build_dock_replan_prompt(
    ctx: &RepoContext,
    violations: &[(String, Vec<super::permissions::Violation>)],
    attempt: usize,
) -> String {
    let base = build_dock_system_prompt(ctx);
    let mut violation_report = String::new();
    for (agent, vs) in violations {
        violation_report.push_str(&format!("\nAgent '{agent}' violated permissions:\n"));
        for v in vs {
            violation_report.push_str(&format!("  - {}: {}\n", v.file_path, v.reason));
        }
    }

    format!(
        "{base}\n\n\
         ## PREVIOUS ATTEMPT FAILED (attempt {attempt})\n\n\
         Your previous plan was rejected due to permission violations:\n\
         {violation_report}\n\
         Please produce a corrected plan. Ensure each agent only targets files within its declared scope."
    )
}

// ---------------------------------------------------------------------------
// Subsystem agent prompts
// ---------------------------------------------------------------------------

/// Build the system prompt for a subsystem agent.
pub fn build_subsystem_agent_prompt(
    ctx: &RepoContext,
    agent_name: &str,
    task: &AgentTask,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // Identity + subsystem descriptions
    sections.push(render_subsystem_identity(ctx, agent_name));

    // Health rollup
    if let Some(health) = render_health_rollup(ctx, agent_name) {
        sections.push(health);
    }

    // File annotations from sidecars
    if let Some(file_anns) = render_file_annotations(ctx, agent_name) {
        sections.push(file_anns);
    }

    // Pending change requests
    if let Some(requests) = render_pending_requests(ctx, agent_name) {
        sections.push(requests);
    }

    // Pickled notes (decisions & context)
    let sidecars = ctx.agent_sidecar_bogs(agent_name);
    if let Some(pickled) = render_pickled_notes(&sidecars) {
        sections.push(pickled);
    }

    // Policies
    if let Some(policies) = render_policies(ctx) {
        sections.push(policies);
    }

    // File boundary (strict)
    sections.push(render_file_boundary(ctx, agent_name));

    // Task + focus files
    sections.push(render_task_section(task));

    // Guidelines
    sections.push(
        "## Guidelines\n\
         - Make targeted, minimal changes to accomplish the task.\n\
         - You may read any file in the repo for context, but only write to your owned files.\n\
         - If you need changes in files you don't own, note this in your output — the orchestrator will handle cross-boundary coordination.\n\
         - Commit your changes when done."
            .to_string(),
    );

    sections.join("\n\n")
}

// ---------------------------------------------------------------------------
// Skimsystem agent prompts
// ---------------------------------------------------------------------------

/// Build the system prompt for a skimsystem agent.
pub fn build_skimsystem_agent_prompt(
    ctx: &RepoContext,
    agent_name: &str,
    task: &AgentTask,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    // Identity + skimsystem descriptions
    sections.push(render_skimsystem_identity(ctx, agent_name));

    // Principles
    sections.push(render_principles(ctx, agent_name));

    // Current skim observations (non-green)
    if let Some(obs) = render_skim_observations(ctx, agent_name) {
        sections.push(obs);
    }

    // Strict boundary + change request format
    sections.push(format!(
        "## STRICT BOUNDARY\n\
         You may ONLY modify *.bog sidecar files. You must NEVER modify source files (*.rs, etc.).\n\
         Your changes should be change_requests addressed to the appropriate subsystem owners.\n\n\
         If you modify any non-.bog file, your entire run will be rejected.\n\n\
         ## Change Request Format\n\
         In .bog files, use this format:\n\
         #[change_requests {{\n  \
         #[request(\n    \
         id = \"unique-id\",\n    \
         from = \"{agent_name}\",\n    \
         target = fn(function_name),\n    \
         type = review,\n    \
         status = pending,\n    \
         created = \"YYYY-MM-DD\",\n    \
         description = \"what needs to change and why\"\n  \
         )]\n\
         }}]"
    ));

    // Task + focus files
    sections.push(render_task_section(task));

    sections.join("\n\n")
}

// ---------------------------------------------------------------------------
// Section renderers — subsystem
// ---------------------------------------------------------------------------

fn render_subsystem_identity(ctx: &RepoContext, agent_name: &str) -> String {
    let subsystem_names = ctx
        .agent_to_subsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    let mut out = format!(
        "You are {agent_name}, a subsystem agent in the bog orchestration system.\n\n\
         ## Your Subsystems"
    );

    for name in &subsystem_names {
        if let Some(sub) = ctx.subsystems.get(name) {
            out.push_str(&format!("\n### {name} ({})", sub.status));
            if let Some(desc) = &sub.description {
                out.push_str(&format!("\n{desc}"));
            }
            let files_str = sub.files.join(", ");
            out.push_str(&format!("\nFiles: {files_str}"));
        }
    }

    out
}

fn render_file_boundary(ctx: &RepoContext, agent_name: &str) -> String {
    let globs = ctx.agent_file_globs(agent_name);
    let glob_list = globs
        .iter()
        .map(|g| format!("- {g}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "## File Boundary (STRICT)\n\
         You may ONLY modify files matching these patterns:\n\
         {glob_list}\n\n\
         STRICT BOUNDARY: If you modify any file outside these patterns, your entire run will be rejected."
    )
}

fn render_file_annotations(ctx: &RepoContext, agent_name: &str) -> Option<String> {
    let sidecars = ctx.agent_sidecar_bogs(agent_name);
    if sidecars.is_empty() {
        return None;
    }

    let mut out = String::from("## File Annotations");

    for (path, bog) in &sidecars {
        let file_status = extract_file_status(bog);
        let status_str = file_status
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        out.push_str(&format!("\n### {path}{status_str}"));

        // Description
        if let Some(desc) = extract_description(bog) {
            let truncated = truncate(&desc, 200);
            out.push_str(&format!("\n{truncated}"));
        }

        // Health dimensions
        if let Some(health_str) = extract_health_line(bog) {
            out.push_str(&format!("\nHealth: {health_str}"));
        }

        // Non-green / stub functions
        let fns = extract_notable_functions(bog);
        if !fns.is_empty() {
            out.push_str("\nNotable functions:");
            for (name, status, desc, is_stub) in &fns {
                let stub_tag = if *is_stub { " [stub]" } else { "" };
                let desc_str = desc
                    .as_deref()
                    .map(|d| format!(" — {d}"))
                    .unwrap_or_default();
                out.push_str(&format!("\n  - {name}: {status}{stub_tag}{desc_str}"));
            }
        }
    }

    Some(out)
}

fn render_pending_requests(ctx: &RepoContext, agent_name: &str) -> Option<String> {
    let sidecars = ctx.agent_sidecar_bogs(agent_name);
    let mut has_any = false;
    let mut out = String::from("## Pending Change Requests");

    for (path, bog) in &sidecars {
        let pending = extract_pending_requests(bog);
        if pending.is_empty() {
            continue;
        }
        has_any = true;
        out.push_str(&format!("\n### {path} ({} pending)", pending.len()));
        for (id, target, desc) in &pending {
            out.push_str(&format!("\n- [{id}] {target}: {desc}"));
        }
    }

    if has_any { Some(out) } else { None }
}

fn render_pickled_notes(sidecars: &[(String, &BogFile)]) -> Option<String> {
    let mut pickled: Vec<(String, String, String, String)> = Vec::new();
    for (_, bog) in sidecars {
        for ann in &bog.annotations {
            if let Annotation::Pickled(p) = ann {
                let tags = p
                    .tags
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let content = truncate(&p.content, 300);
                pickled.push((p.id.clone(), p.kind.to_string(), tags, content));
            }
        }
    }

    if pickled.is_empty() {
        return None;
    }

    // Take max 5, sorted by id as a rough proxy for recency
    pickled.sort_by(|a, b| b.0.cmp(&a.0));
    pickled.truncate(5);

    let mut out = String::from("## Decisions & Context (Pickled Notes)");
    for (id, kind, tags, content) in &pickled {
        let tags_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{tags}]")
        };
        out.push_str(&format!("\n- [{id}] ({kind}){tags_str}: {content}"));
    }

    Some(out)
}

fn render_health_rollup(ctx: &RepoContext, agent_name: &str) -> Option<String> {
    let subsystem_names = ctx
        .agent_to_subsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    if subsystem_names.is_empty() {
        return None;
    }

    let sidecars = ctx.agent_sidecar_bogs(agent_name);
    if sidecars.is_empty() {
        return None;
    }

    let mut out = String::from("## Health Summary");

    for sub_name in &subsystem_names {
        let sub = match ctx.subsystems.get(sub_name) {
            Some(s) => s,
            None => continue,
        };

        // Aggregate health across files in this subsystem
        let sub_sidecars: Vec<&BogFile> = sidecars
            .iter()
            .filter(|(path, _)| sub.files.iter().any(|f| f.as_str() == *path))
            .map(|(_, bog)| *bog)
            .collect();

        if sub_sidecars.is_empty() {
            continue;
        }

        let mut worst = Status::Green;
        let mut dim_worsts: std::collections::HashMap<String, Status> =
            std::collections::HashMap::new();
        let mut fn_counts = (0usize, 0usize, 0usize); // green, yellow, red

        for bog in &sub_sidecars {
            for ann in &bog.annotations {
                match ann {
                    Annotation::Health(h) => {
                        for (dim, status) in &h.dimensions {
                            let entry = dim_worsts.entry(dim.clone()).or_insert(Status::Green);
                            *entry = worse(*entry, *status);
                            worst = worse(worst, *status);
                        }
                    }
                    Annotation::Fn(f) => match f.status {
                        Status::Green => fn_counts.0 += 1,
                        Status::Yellow => {
                            fn_counts.1 += 1;
                            worst = worse(worst, Status::Yellow);
                        }
                        Status::Red => {
                            fn_counts.2 += 1;
                            worst = worse(worst, Status::Red);
                        }
                    },
                    _ => {}
                }
            }
        }

        let dims_str: String = dim_worsts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");

        out.push_str(&format!(
            "\n- {sub_name}: {worst} (worst) — {dims_str} — fns: {}/{}/{} green/yellow/red",
            fn_counts.0, fn_counts.1, fn_counts.2
        ));
    }

    Some(out)
}

fn render_policies(ctx: &RepoContext) -> Option<String> {
    for ann in &ctx.repo_bog.annotations {
        if let Annotation::Policies(p) = ann {
            let mut lines = Vec::new();
            for (key, val) in &p.fields {
                lines.push(format!("- {key}: {}", format_value(val)));
            }
            if lines.is_empty() {
                return None;
            }
            lines.sort();
            return Some(format!("## Policies\n{}", lines.join("\n")));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Section renderers — skimsystem
// ---------------------------------------------------------------------------

fn render_skimsystem_identity(ctx: &RepoContext, agent_name: &str) -> String {
    let skimsystem_names = ctx
        .agent_to_skimsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    let mut out = format!(
        "You are {agent_name}, a skimsystem agent in the bog orchestration system.\n\n\
         ## Your Skimsystems"
    );

    for name in &skimsystem_names {
        if let Some(skim) = ctx.skimsystems.get(name) {
            out.push_str(&format!("\n### {name} ({})", skim.status));
            if let Some(desc) = &skim.description {
                out.push_str(&format!("\n{desc}"));
            }
        }
    }

    out
}

fn render_principles(ctx: &RepoContext, agent_name: &str) -> String {
    let skimsystem_names = ctx
        .agent_to_skimsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    let mut principles = Vec::new();
    for name in &skimsystem_names {
        if let Some(skim) = ctx.skimsystems.get(name) {
            for p in &skim.principles {
                principles.push(format!("- [{name}] {p}"));
            }
        }
    }

    let list = if principles.is_empty() {
        "(none)".to_string()
    } else {
        principles.join("\n")
    };

    format!("## Principles\n{list}")
}

fn render_skim_observations(ctx: &RepoContext, agent_name: &str) -> Option<String> {
    let skimsystem_names = ctx
        .agent_to_skimsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    if skimsystem_names.is_empty() {
        return None;
    }

    // Collect all sidecars across all owned skimsystems
    let mut all_sidecars: Vec<(String, &BogFile)> = Vec::new();
    for skim_name in &skimsystem_names {
        for (path, bog) in ctx.skimsystem_sidecar_bogs(skim_name) {
            if !all_sidecars.iter().any(|(p, _)| p == &path) {
                all_sidecars.push((path, bog));
            }
        }
    }
    all_sidecars.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("## Current Observations (non-green)");
    let mut has_any = false;

    for (path, bog) in &all_sidecars {
        for ann in &bog.annotations {
            if let Annotation::Skim(obs) = ann {
                if skimsystem_names.contains(&obs.skimsystem)
                    && obs.status != Status::Green
                {
                    has_any = true;
                    let notes_str = obs
                        .notes
                        .as_deref()
                        .map(|n| format!(": {n}"))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "\n### {path} — {}{notes_str}",
                        obs.status
                    ));
                }
            }
        }
    }

    if has_any { Some(out) } else { None }
}

// ---------------------------------------------------------------------------
// Section renderers — dock
// ---------------------------------------------------------------------------

fn render_subsystem_summary_rich(ctx: &RepoContext) -> String {
    let mut lines = Vec::new();
    for (name, sub) in &ctx.subsystems {
        let desc = sub
            .description
            .as_deref()
            .map(|d| format!(" — {d}"))
            .unwrap_or_default();
        let files_str = sub.files.join(", ");
        lines.push(format!(
            "- **{name}** (owner: {owner}, status: {status}){desc}\n  files: {files_str}",
            owner = sub.owner,
            status = sub.status,
        ));
    }
    lines.sort();
    lines.join("\n")
}

fn render_skimsystem_summary_rich(ctx: &RepoContext) -> String {
    let mut lines = Vec::new();
    for (name, skim) in &ctx.skimsystems {
        let desc = skim
            .description
            .as_deref()
            .map(|d| format!(" — {d}"))
            .unwrap_or_default();
        let targets_str = format!("{:?}", skim.targets);
        lines.push(format!(
            "- **{name}** (owner: {owner}, status: {status}, targets: {targets_str}){desc}",
            owner = skim.owner,
            status = skim.status,
        ));
    }
    lines.sort();
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Shared section renderers
// ---------------------------------------------------------------------------

fn render_task_section(task: &AgentTask) -> String {
    let focus = if task.focus_files.is_empty() {
        "(none specified)".to_string()
    } else {
        task.focus_files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "## Task\n{instruction}\n\n## Focus Files\n{focus}",
        instruction = task.instruction
    )
}

// ---------------------------------------------------------------------------
// Sidecar data extraction helpers
// ---------------------------------------------------------------------------

fn extract_file_status(bog: &BogFile) -> Option<Status> {
    for ann in &bog.annotations {
        if let Annotation::File(f) = ann {
            return Some(f.status);
        }
    }
    None
}

fn extract_description(bog: &BogFile) -> Option<String> {
    for ann in &bog.annotations {
        if let Annotation::Description(d) = ann {
            return Some(d.clone());
        }
    }
    None
}

fn extract_health_line(bog: &BogFile) -> Option<String> {
    for ann in &bog.annotations {
        if let Annotation::Health(h) = ann {
            if h.dimensions.is_empty() {
                return None;
            }
            let dims: Vec<String> = h
                .dimensions
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            return Some(dims.join(", "));
        }
    }
    None
}

/// Extract functions that are non-green or stubs — the notable ones worth surfacing.
fn extract_notable_functions(bog: &BogFile) -> Vec<(String, Status, Option<String>, bool)> {
    let mut fns = Vec::new();
    for ann in &bog.annotations {
        if let Annotation::Fn(f) = ann {
            if f.status != Status::Green || f.stub {
                fns.push((
                    f.name.clone(),
                    f.status,
                    f.description.clone(),
                    f.stub,
                ));
            }
        }
    }
    fns
}

/// Extract pending change requests as (id, target_str, description).
fn extract_pending_requests(bog: &BogFile) -> Vec<(String, String, String)> {
    let mut requests = Vec::new();
    for ann in &bog.annotations {
        if let Annotation::ChangeRequests(reqs) = ann {
            for r in reqs {
                if r.status == "pending" {
                    requests.push((r.id.clone(), format_target(&r.target), r.description.clone()));
                }
            }
        }
    }
    requests
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

fn worse(a: Status, b: Status) -> Status {
    match (a, b) {
        (Status::Red, _) | (_, Status::Red) => Status::Red,
        (Status::Yellow, _) | (_, Status::Yellow) => Status::Yellow,
        _ => Status::Green,
    }
}

fn format_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Ident(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Status(s) => s.to_string(),
        Value::FnRef(name) => format!("fn({name})"),
        Value::Path(parts) => parts.join("::"),
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Tuple(items) => {
            let inner: Vec<String> = items.iter().map(format_value).collect();
            format!("({})", inner.join(", "))
        }
        Value::Block(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", format_value(v)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
    }
}

fn format_target(v: &Value) -> String {
    match v {
        Value::Ident(s) | Value::String(s) => s.clone(),
        Value::FnRef(name) => format!("fn({name})"),
        Value::Path(parts) => parts.join("::"),
        _ => format!("{v:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn load_ctx() -> RepoContext {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        RepoContext::load(root).unwrap()
    }

    #[test]
    fn test_dock_prompt_contains_agents() {
        let ctx = load_ctx();
        let prompt = build_dock_system_prompt(&ctx);
        assert!(prompt.contains("core-agent"));
        assert!(prompt.contains("analysis-agent"));
        assert!(prompt.contains("cli-agent"));
        assert!(prompt.contains("code-standards-agent"));
        assert!(prompt.contains("observability-agent"));
    }

    #[test]
    fn test_dock_prompt_contains_json_schema() {
        let ctx = load_ctx();
        let prompt = build_dock_system_prompt(&ctx);
        assert!(prompt.contains("\"summary\""));
        assert!(prompt.contains("\"tasks\""));
        assert!(prompt.contains("\"agent\""));
    }

    #[test]
    fn test_subsystem_prompt_contains_globs() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "core-agent".to_string(),
            instruction: "Fix parser bug".to_string(),
            focus_files: vec!["src/parser.rs".to_string()],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_subsystem_agent_prompt(&ctx, "core-agent", &task);
        assert!(prompt.contains("ast.rs"));
        assert!(prompt.contains("parser.rs"));
        assert!(prompt.contains("STRICT BOUNDARY"));
        assert!(prompt.contains("Fix parser bug"));
    }

    #[test]
    fn test_skimsystem_prompt_contains_principles() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "code-standards-agent".to_string(),
            instruction: "Review annotation quality".to_string(),
            focus_files: vec![],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_skimsystem_agent_prompt(&ctx, "code-standards-agent", &task);
        assert!(prompt.contains("ONLY modify *.bog"));
        assert!(prompt.contains("NEVER modify source files"));
        assert!(prompt.contains("change_requests"));
    }

    #[test]
    fn test_replan_prompt_includes_violations() {
        let ctx = load_ctx();
        let violations = vec![(
            "core-agent".to_string(),
            vec![super::super::permissions::Violation {
                file_path: "src/cli.rs".to_string(),
                reason: "outside globs".to_string(),
            }],
        )];
        let prompt = build_dock_replan_prompt(&ctx, &violations, 1);
        assert!(prompt.contains("PREVIOUS ATTEMPT FAILED"));
        assert!(prompt.contains("src/cli.rs"));
        assert!(prompt.contains("outside globs"));
    }

    #[test]
    fn test_subsystem_prompt_contains_description() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "core-agent".to_string(),
            instruction: "test".to_string(),
            focus_files: vec![],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_subsystem_agent_prompt(&ctx, "core-agent", &task);
        // Subsystem description from repo.bog
        assert!(
            prompt.contains("Data model"),
            "Should contain core subsystem description"
        );
    }

    #[test]
    fn test_subsystem_prompt_contains_file_annotations() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "core-agent".to_string(),
            instruction: "test".to_string(),
            focus_files: vec![],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_subsystem_agent_prompt(&ctx, "core-agent", &task);
        // File description from src/parser.rs.bog
        assert!(
            prompt.contains("Pest-based parser"),
            "Should contain file description from sidecar"
        );
        // Health dimensions
        assert!(
            prompt.contains("test_coverage"),
            "Should contain health dimensions"
        );
    }

    #[test]
    fn test_skimsystem_prompt_contains_description() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "code-standards-agent".to_string(),
            instruction: "test".to_string(),
            focus_files: vec![],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_skimsystem_agent_prompt(&ctx, "code-standards-agent", &task);
        assert!(
            prompt.contains("pedantic clippy"),
            "Should contain skimsystem description"
        );
    }

    #[test]
    fn test_skimsystem_prompt_contains_observations() {
        let ctx = load_ctx();
        let task = AgentTask {
            agent: "code-standards-agent".to_string(),
            instruction: "test".to_string(),
            focus_files: vec![],
            depends_on: vec![],
            model: None,
        };
        let prompt = build_skimsystem_agent_prompt(&ctx, "code-standards-agent", &task);
        // Should have non-green skim observations
        assert!(
            prompt.contains("non-green"),
            "Should contain observations section header"
        );
    }

    #[test]
    fn test_dock_prompt_contains_subsystem_descriptions() {
        let ctx = load_ctx();
        let prompt = build_dock_system_prompt(&ctx);
        assert!(
            prompt.contains("Data model"),
            "Dock prompt should contain subsystem descriptions"
        );
    }
}
