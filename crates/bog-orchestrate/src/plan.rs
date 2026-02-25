use serde::{Deserialize, Serialize};

use crate::context::RepoContext;
use crate::error::OrchestrateError;
use crate::permissions::Violation;

/// The structured plan output by the dock agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockPlan {
    pub summary: String,
    pub tasks: Vec<AgentTask>,
}

/// A single task assigned to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Agent name (must match an agent in bog.toml).
    pub agent: String,
    /// What the agent should do — natural language instruction.
    pub instruction: String,
    /// Specific files the agent should focus on.
    #[serde(default)]
    pub focus_files: Vec<String>,
    /// Indices into the `tasks` array that must complete first.
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

/// Result of executing a single agent task.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub agent: String,
    pub task_index: usize,
    pub status: AgentResultStatus,
    pub files_modified: Vec<String>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub enum AgentResultStatus {
    Success,
    Failed(String),
    PermissionViolation(Vec<Violation>),
}

/// Validate a dock plan against the repo context.
pub fn validate_plan(plan: &DockPlan, ctx: &RepoContext) -> Result<(), OrchestrateError> {
    for (i, task) in plan.tasks.iter().enumerate() {
        // Check agent exists in bog.toml
        if !ctx.config.agents.contains_key(&task.agent) {
            return Err(OrchestrateError::InvalidPlan(format!(
                "Task {i}: agent '{}' is not registered in bog.toml",
                task.agent
            )));
        }

        // Check depends_on indices are valid
        for &dep in &task.depends_on {
            if dep >= plan.tasks.len() {
                return Err(OrchestrateError::InvalidPlan(format!(
                    "Task {i}: depends_on index {dep} is out of bounds (only {} tasks)",
                    plan.tasks.len()
                )));
            }
            if dep >= i {
                return Err(OrchestrateError::InvalidPlan(format!(
                    "Task {i}: depends_on index {dep} is not a prior task (would create cycle)"
                )));
            }
        }
    }

    // Check for cycles via topological sort
    topological_sort(plan)?;

    Ok(())
}

/// Topological sort of tasks based on depends_on relationships.
/// Returns execution order (task indices).
pub fn topological_sort(plan: &DockPlan) -> Result<Vec<usize>, OrchestrateError> {
    let n = plan.tasks.len();
    let mut in_degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, task) in plan.tasks.iter().enumerate() {
        in_degree[i] = task.depends_on.len();
        for &dep in &task.depends_on {
            adj[dep].push(i);
        }
    }

    let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(node) = queue.pop() {
        order.push(node);
        for &next in &adj[node] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                queue.push(next);
            }
        }
    }

    if order.len() != n {
        return Err(OrchestrateError::InvalidPlan(
            "Task dependency cycle detected".to_string(),
        ));
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_plan(tasks: Vec<AgentTask>) -> DockPlan {
        DockPlan {
            summary: "test".to_string(),
            tasks,
        }
    }

    fn task(agent: &str, deps: Vec<usize>) -> AgentTask {
        AgentTask {
            agent: agent.to_string(),
            instruction: "do something".to_string(),
            focus_files: vec![],
            depends_on: deps,
        }
    }

    #[test]
    fn test_topological_sort_linear() {
        let plan = mock_plan(vec![
            task("a", vec![]),
            task("b", vec![0]),
            task("c", vec![1]),
        ]);
        let order = topological_sort(&plan).unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn test_topological_sort_parallel() {
        let plan = mock_plan(vec![
            task("a", vec![]),
            task("b", vec![]),
            task("c", vec![0, 1]),
        ]);
        let order = topological_sort(&plan).unwrap();
        // c must come last, a and b can be in any order
        assert_eq!(*order.last().unwrap(), 2);
        assert!(order.contains(&0));
        assert!(order.contains(&1));
    }

    #[test]
    fn test_validate_plan_bad_agent() {
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let ctx = RepoContext::load(root).unwrap();
        let plan = mock_plan(vec![task("nonexistent-agent", vec![])]);
        let err = validate_plan(&plan, &ctx).unwrap_err();
        assert!(err.to_string().contains("nonexistent-agent"));
    }

    #[test]
    fn test_validate_plan_bad_dep_index() {
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let ctx = RepoContext::load(root).unwrap();
        let plan = mock_plan(vec![task("core-agent", vec![5])]);
        let err = validate_plan(&plan, &ctx).unwrap_err();
        assert!(err.to_string().contains("out of bounds"));
    }

    #[test]
    fn test_validate_plan_valid() {
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let ctx = RepoContext::load(root).unwrap();
        let plan = mock_plan(vec![
            task("core-agent", vec![]),
            task("cli-agent", vec![0]),
        ]);
        validate_plan(&plan, &ctx).unwrap();
    }
}
