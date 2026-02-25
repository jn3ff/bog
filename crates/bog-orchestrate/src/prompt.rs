use crate::context::RepoContext;
use crate::plan::AgentTask;

/// Build the system prompt for the dock agent.
pub fn build_dock_system_prompt(ctx: &RepoContext) -> String {
    format!(
        r#"You are the dock agent for the bog orchestration system.

Your role is to analyze user requests and produce a structured execution plan that delegates work to registered agents. You are READ-ONLY — do not modify any files.

## Repository Declarations (repo.bog)

{repo_bog}

## Agent Registry (bog.toml)

{agent_registry}

## Subsystem Ownership

{subsystem_summary}

## Skimsystem Coverage

{skimsystem_summary}

## Rules

1. Subsystem agents can ONLY modify files matching their subsystem's glob patterns.
2. Skimsystem agents can ONLY modify *.bog files (never source files). They create change_requests.
3. Each task must specify a registered agent from bog.toml.
4. Tasks may depend on other tasks using depends_on with task indices.
5. Instructions should be specific and actionable.
6. focus_files should list the specific files the agent should work on.

## Output Format

Respond with ONLY a JSON object matching this schema (no markdown, no explanation):
{{
  "summary": "string — what you plan to do",
  "tasks": [
    {{
      "agent": "string — agent name from bog.toml",
      "instruction": "string — specific instruction for this agent",
      "focus_files": ["string — file paths to focus on"],
      "depends_on": [0]
    }}
  ]
}}"#,
        repo_bog = ctx.repo_bog_raw,
        agent_registry = ctx.format_agent_registry(),
        subsystem_summary = ctx.format_subsystem_summary(),
        skimsystem_summary = ctx.format_skimsystem_summary(),
    )
}

/// Build a replan prompt that includes violation feedback from a previous attempt.
pub fn build_dock_replan_prompt(
    ctx: &RepoContext,
    violations: &[(String, Vec<crate::permissions::Violation>)],
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
        r#"{base}

## PREVIOUS ATTEMPT FAILED (attempt {attempt})

Your previous plan was rejected due to permission violations:
{violation_report}
Please produce a corrected plan. Ensure each agent only targets files within its declared scope."#
    )
}

/// Build the system prompt for a subsystem agent.
pub fn build_subsystem_agent_prompt(
    ctx: &RepoContext,
    agent_name: &str,
    task: &AgentTask,
) -> String {
    let globs = ctx.agent_file_globs(agent_name);
    let subsystem_names = ctx
        .agent_to_subsystems
        .get(agent_name)
        .cloned()
        .unwrap_or_default();

    format!(
        r#"You are {agent_name}, a subsystem agent in the bog orchestration system.

## Your Subsystems
{subsystem_list}

## Files You Own
You may ONLY modify files matching these patterns:
{glob_list}

STRICT BOUNDARY: If you modify any file outside these patterns, your entire run will be rejected.

## Task
{instruction}

## Focus Files
{focus_files}

## Guidelines
- Make targeted, minimal changes to accomplish the task.
- You may read any file in the repo for context, but only write to your owned files.
- If you need changes in files you don't own, note this in your output — the orchestrator will handle cross-boundary coordination.
- Commit your changes when done."#,
        subsystem_list = subsystem_names
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
        glob_list = globs
            .iter()
            .map(|g| format!("- {g}"))
            .collect::<Vec<_>>()
            .join("\n"),
        instruction = task.instruction,
        focus_files = if task.focus_files.is_empty() {
            "(none specified)".to_string()
        } else {
            task.focus_files
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

/// Build the system prompt for a skimsystem agent.
pub fn build_skimsystem_agent_prompt(
    ctx: &RepoContext,
    agent_name: &str,
    task: &AgentTask,
) -> String {
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

    format!(
        r#"You are {agent_name}, a skimsystem agent in the bog orchestration system.

## Your Skimsystems
{skimsystem_list}

## Principles
{principles_list}

## STRICT BOUNDARY
You may ONLY modify *.bog sidecar files. You must NEVER modify source files (*.rs, etc.).
Your changes should be change_requests addressed to the appropriate subsystem owners.

If you modify any non-.bog file, your entire run will be rejected.

## Change Request Format
In .bog files, use this format:
#[change_requests {{
  #[request(
    id = "unique-id",
    from = "{agent_name}",
    target = fn(function_name),
    type = review,
    status = pending,
    created = "YYYY-MM-DD",
    description = "what needs to change and why"
  )]
}}]

## Task
{instruction}

## Focus Files
{focus_files}"#,
        skimsystem_list = skimsystem_names
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
        principles_list = if principles.is_empty() {
            "(none)".to_string()
        } else {
            principles.join("\n")
        },
        instruction = task.instruction,
        focus_files = if task.focus_files.is_empty() {
            "(none specified)".to_string()
        } else {
            task.focus_files
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn load_ctx() -> RepoContext {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        RepoContext::load(root).unwrap()
    }

    #[test]
    fn test_dock_prompt_contains_agents() {
        let ctx = load_ctx();
        let prompt = build_dock_system_prompt(&ctx);
        assert!(prompt.contains("core-agent"));
        assert!(prompt.contains("analysis-agent"));
        assert!(prompt.contains("cli-agent"));
        assert!(prompt.contains("quality-agent"));
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
            focus_files: vec!["crates/bog/src/parser.rs".to_string()],
            depends_on: vec![],
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
            agent: "quality-agent".to_string(),
            instruction: "Review annotation quality".to_string(),
            focus_files: vec![],
            depends_on: vec![],
        };
        let prompt = build_skimsystem_agent_prompt(&ctx, "quality-agent", &task);
        assert!(prompt.contains("ONLY modify *.bog"));
        assert!(prompt.contains("NEVER modify source files"));
        assert!(prompt.contains("change_requests"));
    }

    #[test]
    fn test_replan_prompt_includes_violations() {
        let ctx = load_ctx();
        let violations = vec![(
            "core-agent".to_string(),
            vec![crate::permissions::Violation {
                file_path: "src/cli.rs".to_string(),
                reason: "outside globs".to_string(),
            }],
        )];
        let prompt = build_dock_replan_prompt(&ctx, &violations, 1);
        assert!(prompt.contains("PREVIOUS ATTEMPT FAILED"));
        assert!(prompt.contains("src/cli.rs"));
        assert!(prompt.contains("outside globs"));
    }
}
