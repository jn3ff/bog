use std::collections::HashMap;
use std::process::Command;

use crate::ast::{Annotation, ChangeRequest};
use crate::parser;

use super::agent;
use super::context::RepoContext;
use super::error::OrchestrateError;
use super::permissions::Violation;
use super::plan::{AgentResult, AgentResultStatus, AgentTask};
use super::provider::Provider;
use super::worktree::WorktreeManager;

/// A group of pending change_requests targeting a single subsystem.
#[derive(Debug)]
pub struct SubsystemWorkPacket {
    pub subsystem: String,
    pub agent: String,
    /// (bog_file_path, source_file_path, Vec<ChangeRequest>)
    pub requests: Vec<(String, String, Vec<ChangeRequest>)>,
}

/// Result of a full skim orchestration run.
pub struct SkimRunResult {
    pub skimsystem: String,
    pub integration_output: String,
    pub work_packets: Vec<SubsystemWorkPacket>,
    pub agent_results: Vec<AgentResult>,
    pub merged: bool,
    pub violations: Vec<(String, Vec<Violation>)>,
}

/// Run a complete skim lifecycle:
/// 1. Execute the skimsystem integration (generates change_requests)
/// 2. Collect pending change_requests grouped by subsystem
/// 3. Delegate to subsystem agents
/// 4. Validate and merge
/// 5. Skimsystem agent closes out
pub fn run_skim_lifecycle(
    ctx: &RepoContext,
    skimsystem_name: &str,
    action: Option<&str>,
    provider: &dyn Provider,
) -> Result<SkimRunResult, OrchestrateError> {
    let run_id = uuid::Uuid::new_v4().to_string();

    // Phase 1: Run the integration via bog CLI
    eprintln!("[skim] Phase 1: Running {skimsystem_name} integration...");
    let integration_output = run_bog_skim(ctx, skimsystem_name, action)?;
    eprintln!("{integration_output}");

    // Phase 2: Collect pending change_requests
    eprintln!("[skim] Phase 2: Collecting pending change_requests...");
    let work_packets = collect_pending_requests(ctx, skimsystem_name)?;

    if work_packets.is_empty() {
        eprintln!("[skim] No pending change_requests found. Nothing to delegate.");
        return Ok(SkimRunResult {
            skimsystem: skimsystem_name.to_string(),
            integration_output,
            work_packets: vec![],
            agent_results: vec![],
            merged: true,
            violations: vec![],
        });
    }

    for wp in &work_packets {
        let total: usize = wp.requests.iter().map(|(_, _, rs)| rs.len()).sum();
        eprintln!(
            "[skim]   {} ({}) — {} change_requests across {} files",
            wp.subsystem,
            wp.agent,
            total,
            wp.requests.len()
        );
    }

    // Phase 3: Delegate to subsystem agents
    eprintln!("[skim] Phase 3: Delegating to subsystem agents...");
    let mut worktree_mgr = WorktreeManager::new(&ctx.root);
    let mut agent_results: Vec<AgentResult> = Vec::new();
    let mut all_violations: Vec<(String, Vec<Violation>)> = Vec::new();
    let mut any_failed = false;

    for (i, wp) in work_packets.iter().enumerate() {
        let task = build_subsystem_task_from_requests(wp);
        eprintln!(
            "[skim]   Spawning {} for subsystem '{}'...",
            wp.agent, wp.subsystem
        );

        let worktree = worktree_mgr
            .create_worktree(&wp.agent, &run_id)
            .map_err(OrchestrateError::Worktree)?;

        let result = match agent::execute_agent_task(ctx, &task, i, worktree, provider) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[skim]   {} error: {e}", wp.agent);
                // Clean up worktrees before propagating
                let _ = worktree_mgr.cleanup_run(&run_id);
                return Err(e);
            }
        };

        match &result.status {
            AgentResultStatus::Success => {
                eprintln!(
                    "[skim]   {} succeeded — {} files modified",
                    wp.agent,
                    result.files_modified.len()
                );
            }
            AgentResultStatus::Failed(msg) => {
                eprintln!("[skim]   {} failed: {msg}", wp.agent);
                any_failed = true;
            }
            AgentResultStatus::PermissionViolation(vs) => {
                eprintln!("[skim]   {} permission violations: {}", wp.agent, vs.len());
                all_violations.push((wp.agent.clone(), vs.clone()));
                any_failed = true;
            }
        }

        agent_results.push(result);
    }

    // Phase 4: Merge or reject
    let merged = if !any_failed && all_violations.is_empty() {
        eprintln!("[skim] Phase 4: Merging agent changes...");
        for wp in &work_packets {
            if let Some(wt) = worktree_mgr.find_worktree(&wp.agent, &run_id) {
                worktree_mgr
                    .merge_changes(wt)
                    .map_err(OrchestrateError::Worktree)?;
            }
        }
        true
    } else {
        eprintln!("[skim] Phase 4: Rejecting — violations or failures detected.");
        false
    };

    // Cleanup
    worktree_mgr
        .cleanup_run(&run_id)
        .map_err(OrchestrateError::Worktree)?;

    Ok(SkimRunResult {
        skimsystem: skimsystem_name.to_string(),
        integration_output,
        work_packets,
        agent_results,
        merged,
        violations: all_violations,
    })
}

/// Run `bog skim` via subprocess to generate change_requests.
fn run_bog_skim(
    ctx: &RepoContext,
    skimsystem_name: &str,
    action: Option<&str>,
) -> Result<String, OrchestrateError> {
    let mut cmd = Command::new("cargo");
    cmd.args(["run", "--", "skim", "."]);
    cmd.args(["--name", skimsystem_name]);
    if let Some(action) = action {
        cmd.args(["--action", action]);
    }
    cmd.current_dir(&ctx.root);

    let output = cmd
        .output()
        .map_err(|e| OrchestrateError::ContextLoad(format!("cargo run bog skim: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // bog skim outputs to stdout; cargo noise goes to stderr
    Ok(format!("{stdout}{stderr}"))
}

/// Scan all .bog files for pending change_requests from a given skimsystem,
/// grouped by owning subsystem.
fn collect_pending_requests(
    ctx: &RepoContext,
    skimsystem_name: &str,
) -> Result<Vec<SubsystemWorkPacket>, OrchestrateError> {
    let mut by_subsystem: HashMap<String, Vec<(String, String, Vec<ChangeRequest>)>> =
        HashMap::new();

    // Glob all .bog files under the repo root
    let pattern = ctx.root.join("**/*.rs.bog");
    let pattern_str = pattern.to_string_lossy();

    let paths: Vec<_> = glob::glob(&pattern_str)
        .map_err(|e| OrchestrateError::ContextLoad(format!("glob: {e}")))?
        .filter_map(|p| p.ok())
        .collect();

    for bog_path in paths {
        let content = match std::fs::read_to_string(&bog_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let bog_file = match parser::parse_bog(&content) {
            Ok(f) => f,
            Err(_) => continue,
        };

        // Find the file annotation to get subsystem
        let file_ann = bog_file.annotations.iter().find_map(|a| {
            if let Annotation::File(f) = a {
                Some(f.clone())
            } else {
                None
            }
        });

        let Some(file_ann) = file_ann else {
            continue;
        };

        // Find the skimsystem's owner agent to match change_request `from` field
        let skim_owner = ctx
            .skimsystems
            .get(skimsystem_name)
            .map(|s| s.owner.as_str())
            .unwrap_or("");

        // Collect pending change_requests from this skimsystem's owner
        let pending: Vec<ChangeRequest> = bog_file
            .annotations
            .iter()
            .filter_map(|a| {
                if let Annotation::ChangeRequests(reqs) = a {
                    Some(reqs.clone())
                } else {
                    None
                }
            })
            .flatten()
            .filter(|r| r.status == "pending" && r.from == skim_owner)
            .collect();

        if pending.is_empty() {
            continue;
        }

        // Derive the source file path from the .bog path (strip .bog suffix)
        let bog_str = bog_path.to_string_lossy().to_string();
        let source_path = bog_str.strip_suffix(".bog").unwrap_or(&bog_str).to_string();

        // Make paths relative to repo root
        let rel_bog = pathdiff(&bog_str, &ctx.root.to_string_lossy());
        let rel_source = pathdiff(&source_path, &ctx.root.to_string_lossy());

        by_subsystem
            .entry(file_ann.subsystem.clone())
            .or_default()
            .push((rel_bog, rel_source, pending));
    }

    // Convert to work packets
    let mut packets = Vec::new();
    for (subsystem_name, requests) in by_subsystem {
        let owner = ctx
            .subsystems
            .get(&subsystem_name)
            .map(|s| s.owner.clone())
            .unwrap_or_else(|| "unknown".to_string());

        packets.push(SubsystemWorkPacket {
            subsystem: subsystem_name,
            agent: owner,
            requests,
        });
    }

    // Sort for deterministic ordering
    packets.sort_by(|a, b| a.subsystem.cmp(&b.subsystem));

    Ok(packets)
}

/// Build an AgentTask from a subsystem's pending change_requests.
/// The task instruction contains all the specific issues to fix.
fn build_subsystem_task_from_requests(wp: &SubsystemWorkPacket) -> AgentTask {
    let mut instruction = format!(
        "Fix the following clippy/lint warnings in the {} subsystem. \
         For each issue, fix the code in the source file. \
         After fixing each issue, update the corresponding change_request \
         in the .bog sidecar file: change `status = pending` to `status = resolved`. \
         If you cannot fix an issue, change its status to `status = denied` and add a note.\n\n",
        wp.subsystem
    );

    let mut focus_files = Vec::new();

    for (bog_path, source_path, requests) in &wp.requests {
        instruction.push_str(&format!("### {source_path}\n"));
        focus_files.push(source_path.clone());
        focus_files.push(bog_path.clone());

        for req in requests {
            instruction.push_str(&format!(
                "- [{}] {}\n",
                req.id, req.description
            ));
        }
        instruction.push('\n');
    }

    AgentTask {
        agent: wp.agent.clone(),
        instruction,
        focus_files,
        depends_on: vec![],
        model: None,
    }
}

/// Simple relative path computation.
fn pathdiff(path: &str, base: &str) -> String {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    if path.starts_with(&base) {
        path[base.len()..].to_string()
    } else {
        path.to_string()
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
    fn test_collect_pending_requests() {
        let ctx = load_ctx();
        // This will find change_requests if the skim has been run
        let packets = collect_pending_requests(&ctx, "code-quality").unwrap();
        // We can't assert specific counts since it depends on state,
        // but verify the structure is sound
        for wp in &packets {
            assert!(!wp.agent.is_empty());
            assert!(!wp.subsystem.is_empty());
            for (bog, src, reqs) in &wp.requests {
                assert!(bog.ends_with(".bog"));
                assert!(!src.is_empty());
                for req in reqs {
                    assert_eq!(req.status, "pending");
                }
            }
        }
    }

    #[test]
    fn test_build_task_from_requests() {
        let wp = SubsystemWorkPacket {
            subsystem: "core".to_string(),
            agent: "core-agent".to_string(),
            requests: vec![(
                "src/parser.rs.bog".to_string(),
                "src/parser.rs".to_string(),
                vec![ChangeRequest {
                    id: "test-123".to_string(),
                    from: "code-standards-agent".to_string(),
                    target: crate::ast::Value::FnRef("parse_bog".to_string()),
                    change_type: "lint_warning".to_string(),
                    status: "pending".to_string(),
                    priority: None,
                    created: "2026-02-25".to_string(),
                    description: "clippy::needless_pass_by_value (line 42): argument passed by value".to_string(),
                }],
            )],
        };

        let task = build_subsystem_task_from_requests(&wp);
        assert_eq!(task.agent, "core-agent");
        assert!(task.instruction.contains("parser.rs"));
        assert!(task.instruction.contains("needless_pass_by_value"));
        assert!(task.instruction.contains("status = pending"));
        assert!(task.focus_files.contains(&"src/parser.rs".to_string()));
        assert!(task.focus_files.contains(&"src/parser.rs.bog".to_string()));
    }
}
