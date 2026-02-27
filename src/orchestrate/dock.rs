use super::context::RepoContext;
use super::error::OrchestrateError;
use super::permissions::Violation;
use super::plan::{self, DockPlan};
use super::prompt;
use super::provider::{Provider, ProviderOptions};

/// Context provided to the dock for replanning after violations.
pub struct ReplanContext {
    pub previous_plan: DockPlan,
    pub violations: Vec<(String, Vec<Violation>)>,
    pub attempt_number: usize,
}

/// Invoke the dock agent to analyze a request and produce a plan.
pub fn run_dock(
    ctx: &RepoContext,
    user_request: &str,
    provider: &dyn Provider,
    replan_context: Option<&ReplanContext>,
) -> Result<DockPlan, OrchestrateError> {
    let system_prompt = if let Some(replan) = replan_context {
        prompt::build_dock_replan_prompt(ctx, &replan.violations, replan.attempt_number)
    } else {
        prompt::build_dock_system_prompt(ctx)
    };

    let options = ProviderOptions {
        read_only: true,
        allowed_tools: Some(vec![
            "Read".to_string(),
            "Grep".to_string(),
            "Glob".to_string(),
            "Bash".to_string(),
        ]),
        timeout_seconds: 120,
        agent_label: Some("dock".to_string()),
        ..Default::default()
    };

    let output = provider.invoke(user_request, &system_prompt, &ctx.root, &options)?;

    if output.exit_code != 0 {
        return Err(OrchestrateError::DockFailed(format!(
            "exit code {}: {}",
            output.exit_code, output.stderr
        )));
    }

    let plan = parse_dock_output(&output.stdout)?;

    plan::validate_plan(&plan, ctx)?;

    Ok(plan)
}

/// Parse the dock agent's output to extract a DockPlan.
/// Handles both raw JSON and Claude CLI JSON wrapper (with `result` field).
fn parse_dock_output(stdout: &str) -> Result<DockPlan, OrchestrateError> {
    let stdout = stdout.trim();

    // Try parsing as Claude CLI JSON output (has a `result` field)
    if let Ok(cli_output) = serde_json::from_str::<serde_json::Value>(stdout) {
        if let Some(result) = cli_output.get("result").and_then(|v| v.as_str()) {
            // The result field contains the actual plan as a string
            if let Ok(plan) = extract_json_plan(result) {
                return Ok(plan);
            }
        }
        // Maybe the whole thing is the plan directly
        if let Ok(plan) = serde_json::from_value::<DockPlan>(cli_output) {
            return Ok(plan);
        }
    }

    // Try extracting JSON from markdown code blocks or raw text
    extract_json_plan(stdout)
}

/// Try to extract a DockPlan JSON from text that may contain markdown or other wrapping.
fn extract_json_plan(text: &str) -> Result<DockPlan, OrchestrateError> {
    let text = text.trim();

    // Direct JSON parse
    if let Ok(plan) = serde_json::from_str::<DockPlan>(text) {
        return Ok(plan);
    }

    // Try to find JSON in markdown code blocks
    if let Some(start) = text.find("```json") {
        let after_fence = &text[start + 7..];
        if let Some(end) = after_fence.find("```") {
            let json_str = after_fence[..end].trim();
            if let Ok(plan) = serde_json::from_str::<DockPlan>(json_str) {
                return Ok(plan);
            }
        }
    }

    // Try to find any JSON object in the text
    if let Some(start) = text.find('{') {
        // Find matching closing brace
        let mut depth = 0;
        let mut end = start;
        for (i, ch) in text[start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth == 0 {
            let json_str = &text[start..end];
            if let Ok(plan) = serde_json::from_str::<DockPlan>(json_str) {
                return Ok(plan);
            }
        }
    }

    Err(OrchestrateError::DockFailed(format!(
        "Could not parse DockPlan from output: {text}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_raw_json() {
        let json = r#"{"summary":"test","tasks":[{"agent":"core-agent","instruction":"do it"}]}"#;
        let plan = parse_dock_output(json).unwrap();
        assert_eq!(plan.summary, "test");
        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].agent, "core-agent");
    }

    #[test]
    fn test_parse_claude_cli_wrapped() {
        let json = r#"{"result":"{\"summary\":\"test\",\"tasks\":[{\"agent\":\"core-agent\",\"instruction\":\"do it\"}]}"}"#;
        let plan = parse_dock_output(json).unwrap();
        assert_eq!(plan.summary, "test");
    }

    #[test]
    fn test_parse_markdown_wrapped() {
        let text = r#"Here is the plan:

```json
{"summary":"test","tasks":[{"agent":"core-agent","instruction":"do it"}]}
```

Done."#;
        let plan = parse_dock_output(text).unwrap();
        assert_eq!(plan.summary, "test");
    }

    #[test]
    fn test_parse_with_surrounding_text() {
        let text = r#"I'll create a plan: {"summary":"test","tasks":[{"agent":"core-agent","instruction":"do it"}]} and that's it."#;
        let plan = parse_dock_output(text).unwrap();
        assert_eq!(plan.summary, "test");
    }

    #[test]
    fn test_parse_invalid() {
        let result = parse_dock_output("not json at all");
        assert!(result.is_err());
    }
}
