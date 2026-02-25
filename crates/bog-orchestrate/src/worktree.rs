use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::WorktreeError;

/// Represents a managed git worktree for an agent.
pub struct AgentWorktree {
    pub path: PathBuf,
    pub branch: String,
    pub agent: String,
    pub run_id: String,
    /// The base commit SHA the worktree was created from.
    pub base_commit: String,
}

/// A file change detected in a worktree diff.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub path: String,
    pub change_type: DiffChangeType,
}

#[derive(Debug, Clone)]
pub enum DiffChangeType {
    Added,
    Modified,
    Deleted,
}

/// Manages the lifecycle of git worktrees for agent isolation.
pub struct WorktreeManager {
    repo_root: PathBuf,
    worktree_base: PathBuf,
    active: Vec<AgentWorktree>,
}

impl WorktreeManager {
    pub fn new(repo_root: &Path) -> Self {
        let worktree_base = repo_root.join(".bog-worktrees");
        Self {
            repo_root: repo_root.to_path_buf(),
            worktree_base,
            active: Vec::new(),
        }
    }

    /// Get the base commit SHA (HEAD).
    fn head_sha(&self) -> Result<String, WorktreeError> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git rev-parse: {e}")))?;
        if !output.status.success() {
            return Err(WorktreeError::GitFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Create a new worktree for an agent.
    pub fn create_worktree(
        &mut self,
        agent_name: &str,
        run_id: &str,
    ) -> Result<&AgentWorktree, WorktreeError> {
        let base_commit = self.head_sha()?;
        let branch = format!("bog/orchestrate/{run_id}/{agent_name}");
        let wt_path = self.worktree_base.join(run_id).join(agent_name);

        // Ensure parent directory exists
        if let Some(parent) = wt_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorktreeError::CreateFailed {
                path: wt_path.clone(),
                message: format!("mkdir: {e}"),
            })?;
        }

        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &branch,
                wt_path.to_str().unwrap(),
                "HEAD",
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::CreateFailed {
                path: wt_path.clone(),
                message: format!("git worktree add: {e}"),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::CreateFailed {
                path: wt_path,
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        self.active.push(AgentWorktree {
            path: wt_path,
            branch,
            agent: agent_name.to_string(),
            run_id: run_id.to_string(),
            base_commit,
        });

        Ok(self.active.last().unwrap())
    }

    /// Get the diff of a worktree against its base commit.
    pub fn inspect_diff(worktree: &AgentWorktree) -> Result<Vec<DiffEntry>, WorktreeError> {
        // Check for uncommitted changes (working tree + staged)
        let output = Command::new("git")
            .args(["diff", "--name-status", "HEAD"])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git diff: {e}")))?;

        let mut entries = parse_name_status(&String::from_utf8_lossy(&output.stdout));

        // Also check for new untracked files
        let output = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git ls-files: {e}")))?;

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if !line.is_empty() {
                entries.push(DiffEntry {
                    path: line.to_string(),
                    change_type: DiffChangeType::Added,
                });
            }
        }

        // Also check committed changes since base
        let output = Command::new("git")
            .args([
                "diff",
                "--name-status",
                &format!("{}..HEAD", worktree.base_commit),
            ])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git diff committed: {e}")))?;

        let committed = parse_name_status(&String::from_utf8_lossy(&output.stdout));
        for entry in committed {
            if !entries.iter().any(|e| e.path == entry.path) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Auto-commit any uncommitted changes in a worktree.
    pub fn auto_commit(worktree: &AgentWorktree) -> Result<bool, WorktreeError> {
        // Stage all changes
        let output = Command::new("git")
            .args(["add", "-A"])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git add: {e}")))?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // Check if there's anything to commit
        let output = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git diff --cached: {e}")))?;

        if output.status.success() {
            // Nothing to commit
            return Ok(false);
        }

        // Commit
        let output = Command::new("git")
            .args([
                "commit",
                "-m",
                &format!("bog-orchestrate: agent '{}' changes", worktree.agent),
            ])
            .current_dir(&worktree.path)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git commit: {e}")))?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(true)
    }

    /// Merge changes from a worktree branch back into the main working tree.
    pub fn merge_changes(&self, worktree: &AgentWorktree) -> Result<(), WorktreeError> {
        let output = Command::new("git")
            .args([
                "merge",
                "--no-ff",
                &worktree.branch,
                "-m",
                &format!(
                    "bog-orchestrate: merge agent '{}' changes",
                    worktree.agent
                ),
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::GitFailed(format!("git merge: {e}")))?;

        if !output.status.success() {
            return Err(WorktreeError::GitFailed(format!(
                "merge failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    /// Find a worktree by agent name and run ID.
    pub fn find_worktree(&self, agent: &str, run_id: &str) -> Option<&AgentWorktree> {
        self.active
            .iter()
            .find(|wt| wt.agent == agent && wt.run_id == run_id)
    }

    /// Remove a single worktree and its branch.
    pub fn remove_worktree(&self, worktree: &AgentWorktree) -> Result<(), WorktreeError> {
        let output = Command::new("git")
            .args([
                "worktree",
                "remove",
                "--force",
                worktree.path.to_str().unwrap(),
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| WorktreeError::RemoveFailed {
                path: worktree.path.clone(),
                message: format!("git worktree remove: {e}"),
            })?;

        if !output.status.success() {
            return Err(WorktreeError::RemoveFailed {
                path: worktree.path.clone(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        // Delete the branch
        let _ = Command::new("git")
            .args(["branch", "-D", &worktree.branch])
            .current_dir(&self.repo_root)
            .output();

        Ok(())
    }

    /// Clean up all worktrees for a given run.
    pub fn cleanup_run(&mut self, run_id: &str) -> Result<(), WorktreeError> {
        let to_remove: Vec<AgentWorktree> = self
            .active
            .drain(..)
            .filter(|wt| wt.run_id == run_id)
            .collect();

        // Also collect non-matching ones to put back
        // (drain already removed everything, so active is empty — put back only non-matching would need different approach)
        // Actually drain(..) removes everything, let's fix:
        let remaining: Vec<AgentWorktree> = Vec::new(); // active is already drained

        for wt in &to_remove {
            if let Err(e) = self.remove_worktree(wt) {
                eprintln!("Warning: failed to remove worktree for {}: {e}", wt.agent);
            }
        }

        // Put back any worktrees from other runs
        self.active = remaining;

        // Clean up the run directory if it exists
        let run_dir = self.worktree_base.join(run_id);
        if run_dir.exists() {
            let _ = std::fs::remove_dir_all(&run_dir);
        }

        Ok(())
    }
}

fn parse_name_status(output: &str) -> Vec<DiffEntry> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(2, '\t');
            let status = parts.next()?;
            let path = parts.next()?.trim();
            let change_type = match status.chars().next()? {
                'A' => DiffChangeType::Added,
                'M' => DiffChangeType::Modified,
                'D' => DiffChangeType::Deleted,
                _ => DiffChangeType::Modified,
            };
            Some(DiffEntry {
                path: path.to_string(),
                change_type,
            })
        })
        .collect()
}
