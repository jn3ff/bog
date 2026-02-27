use crate::config::AgentRole;

use super::context::RepoContext;
use super::error::OrchestrateError;
use super::permissions;
use super::plan::{AgentResult, AgentResultStatus, AgentTask};
use super::prompt;
use super::provider::{Provider, ProviderOptions};
use super::worktree::{AgentWorktree, WorktreeManager};

/// Execute a single agent task in an isolated worktree.
pub fn execute_agent_task(
    ctx: &RepoContext,
    task: &AgentTask,
    task_index: usize,
    worktree: &AgentWorktree,
    provider: &dyn Provider,
) -> Result<AgentResult, OrchestrateError> {
    let role = ctx.agent_role(&task.agent).ok_or_else(|| {
        OrchestrateError::AgentFailed {
            agent: task.agent.clone(),
            message: "Agent not declared in repo.bog".into(),
        }
    })?;

    let system_prompt = match role {
        AgentRole::Subsystem => prompt::build_subsystem_agent_prompt(ctx, &task.agent, task),
        AgentRole::Skimsystem => prompt::build_skimsystem_agent_prompt(ctx, &task.agent, task),
    };

    let options = ProviderOptions {
        read_only: false,
        allowed_tools: Some(vec![
            "Bash".to_string(),
            "Edit".to_string(),
            "Read".to_string(),
            "Write".to_string(),
            "Grep".to_string(),
            "Glob".to_string(),
        ]),
        timeout_seconds: 300,
        agent_label: Some(task.agent.clone()),
        ..Default::default()
    };

    let output = provider.invoke(&task.instruction, &system_prompt, &worktree.path, &options)?;

    // Auto-commit any uncommitted changes in the worktree
    WorktreeManager::auto_commit(worktree).map_err(|e| OrchestrateError::AgentFailed {
        agent: task.agent.clone(),
        message: format!("auto-commit failed: {e}"),
    })?;

    // Inspect what changed
    let diff = WorktreeManager::inspect_diff(worktree)
        .map_err(|e| OrchestrateError::AgentFailed {
            agent: task.agent.clone(),
            message: format!("diff inspect failed: {e}"),
        })?;

    // Check permissions
    let violations = permissions::check_agent_permissions(&task.agent, &diff, ctx);

    let files_modified: Vec<String> = diff.iter().map(|d| d.path.clone()).collect();

    if violations.is_empty() {
        Ok(AgentResult {
            agent: task.agent.clone(),
            task_index,
            status: AgentResultStatus::Success,
            files_modified,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    } else {
        Ok(AgentResult {
            agent: task.agent.clone(),
            task_index,
            status: AgentResultStatus::PermissionViolation(violations),
            files_modified,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}
