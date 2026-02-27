use super::agent;
use super::context::RepoContext;
use super::dock::{self, ReplanContext};
use super::error::OrchestrateError;
use super::permissions::Violation;
use super::plan::{self, AgentResult, AgentResultStatus, DockPlan};
use super::provider::Provider;
use super::worktree::WorktreeManager;

/// Configuration for an orchestration run.
pub struct OrchestrateConfig {
    pub max_replan_attempts: usize,
    pub merge_strategy: MergeStrategy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MergeStrategy {
    /// Merge each agent's changes as they succeed.
    Incremental,
    /// Only merge all changes after all agents succeed.
    AllOrNothing,
}

impl Default for OrchestrateConfig {
    fn default() -> Self {
        Self {
            max_replan_attempts: 2,
            merge_strategy: MergeStrategy::AllOrNothing,
        }
    }
}

/// Result of a complete orchestration run.
pub struct OrchestrateResult {
    pub plan: DockPlan,
    pub agent_results: Vec<AgentResult>,
    pub merged: bool,
    pub violations: Vec<(String, Vec<Violation>)>,
}

/// Execute the full orchestration: dock → plan → delegate → validate → merge/reject.
pub fn orchestrate(
    ctx: &RepoContext,
    user_request: &str,
    provider: &dyn Provider,
    config: &OrchestrateConfig,
) -> Result<OrchestrateResult, OrchestrateError> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut worktree_mgr = WorktreeManager::new(&ctx.root);
    let mut replan_context: Option<ReplanContext> = None;

    for attempt in 0..=config.max_replan_attempts {
        eprintln!(
            "[orchestrate] attempt {}/{}",
            attempt + 1,
            config.max_replan_attempts + 1
        );

        // Phase 1: Dock — produce plan
        let plan_result = dock::run_dock(ctx, user_request, provider, replan_context.as_ref());
        let plan = match plan_result {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[orchestrate] dock failed: {e}");
                return Err(e);
            }
        };

        eprintln!(
            "[orchestrate] dock plan: {} ({} tasks)",
            plan.summary,
            plan.tasks.len()
        );

        // Phase 2: Execute agent tasks in dependency order
        let execution_order = plan::topological_sort(&plan)?;
        let mut agent_results: Vec<AgentResult> = Vec::new();
        let mut all_violations: Vec<(String, Vec<Violation>)> = Vec::new();
        let mut any_failed = false;

        for &task_idx in &execution_order {
            let task = &plan.tasks[task_idx];
            eprintln!("[orchestrate] executing task {task_idx}: agent={}", task.agent);

            // Create worktree for this agent
            let worktree = worktree_mgr.create_worktree(&task.agent, &run_id)?;

            // Execute the task
            let result = agent::execute_agent_task(ctx, task, task_idx, worktree, provider)?;

            match &result.status {
                AgentResultStatus::Success => {
                    eprintln!(
                        "[orchestrate] agent '{}' succeeded, {} files modified",
                        result.agent,
                        result.files_modified.len()
                    );
                    if config.merge_strategy == MergeStrategy::Incremental {
                        if let Some(wt) = worktree_mgr.find_worktree(&result.agent, &run_id) {
                            worktree_mgr.merge_changes(wt)?;
                        }
                    }
                }
                AgentResultStatus::Failed(msg) => {
                    eprintln!("[orchestrate] agent '{}' failed: {msg}", result.agent);
                    any_failed = true;
                }
                AgentResultStatus::PermissionViolation(violations) => {
                    eprintln!(
                        "[orchestrate] agent '{}' permission violations: {} files",
                        result.agent,
                        violations.len()
                    );
                    all_violations.push((result.agent.clone(), violations.clone()));
                    any_failed = true;
                }
            }

            agent_results.push(result);

            if any_failed {
                break;
            }
        }

        // Phase 3: Handle results
        if all_violations.is_empty() && !any_failed {
            // All agents succeeded
            if config.merge_strategy == MergeStrategy::AllOrNothing {
                for result in &agent_results {
                    if matches!(result.status, AgentResultStatus::Success) {
                        if let Some(wt) = worktree_mgr.find_worktree(&result.agent, &run_id) {
                            worktree_mgr.merge_changes(wt)?;
                        }
                    }
                }
            }

            worktree_mgr.cleanup_run(&run_id)?;

            return Ok(OrchestrateResult {
                plan,
                agent_results,
                merged: true,
                violations: vec![],
            });
        }

        // Violations or failure — reject entire run
        eprintln!("[orchestrate] rejecting run, cleaning up worktrees");
        worktree_mgr.cleanup_run(&run_id)?;

        if attempt < config.max_replan_attempts && !all_violations.is_empty() {
            replan_context = Some(ReplanContext {
                previous_plan: plan,
                violations: all_violations,
                attempt_number: attempt + 1,
            });
            continue;
        }

        // Exhausted attempts or non-violation failure
        return Ok(OrchestrateResult {
            plan,
            agent_results,
            merged: false,
            violations: all_violations,
        });
    }

    Err(OrchestrateError::ReplanExhausted {
        attempts: config.max_replan_attempts,
    })
}
