use bog::config::AgentRole;

use crate::context::RepoContext;
use crate::worktree::DiffEntry;

/// Record of a file modified outside the agent's allowed scope.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Violation {
    pub file_path: String,
    pub reason: String,
}

/// Check whether an agent's diff is within its allowed permissions.
pub fn check_agent_permissions(
    agent_name: &str,
    diff_entries: &[DiffEntry],
    ctx: &RepoContext,
) -> Vec<Violation> {
    let role = ctx.agent_role(agent_name);
    let mut violations = Vec::new();

    match role {
        Some(AgentRole::Subsystem) => {
            let allowed_globs = ctx.agent_file_globs(agent_name);
            for entry in diff_entries {
                if !matches_any_glob(&entry.path, &allowed_globs) {
                    violations.push(Violation {
                        file_path: entry.path.clone(),
                        reason: format!(
                            "Subsystem agent '{agent_name}' modified '{}' outside its declared globs",
                            entry.path
                        ),
                    });
                }
            }
        }
        Some(AgentRole::Skimsystem) => {
            for entry in diff_entries {
                if !entry.path.ends_with(".bog") {
                    violations.push(Violation {
                        file_path: entry.path.clone(),
                        reason: format!(
                            "Skimsystem agent '{agent_name}' modified non-.bog file '{}'",
                            entry.path
                        ),
                    });
                }
            }
        }
        None => {
            for entry in diff_entries {
                violations.push(Violation {
                    file_path: entry.path.clone(),
                    reason: format!("Agent '{agent_name}' is not registered in bog.toml"),
                });
            }
        }
    }

    violations
}

/// Check if a file path matches any of the given glob patterns.
fn matches_any_glob(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|p| p.matches(path))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::DiffChangeType;

    fn diff(path: &str) -> DiffEntry {
        DiffEntry {
            path: path.to_string(),
            change_type: DiffChangeType::Modified,
        }
    }

    fn load_ctx() -> RepoContext {
        use std::path::Path;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        RepoContext::load(root).unwrap()
    }

    #[test]
    fn test_subsystem_agent_allowed_files() {
        let ctx = load_ctx();
        let entries = vec![diff("crates/bog/src/ast.rs"), diff("crates/bog/src/parser.rs")];
        let violations = check_agent_permissions("core-agent", &entries, &ctx);
        assert!(violations.is_empty(), "core-agent owns these files: {violations:?}");
    }

    #[test]
    fn test_subsystem_agent_disallowed_files() {
        let ctx = load_ctx();
        let entries = vec![diff("crates/bog/src/cli.rs")];
        let violations = check_agent_permissions("core-agent", &entries, &ctx);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.contains("outside its declared globs"));
    }

    #[test]
    fn test_skimsystem_agent_allowed_bog_files() {
        let ctx = load_ctx();
        let entries = vec![diff("crates/bog/src/ast.rs.bog"), diff("crates/bog/src/cli.rs.bog")];
        let violations = check_agent_permissions("quality-agent", &entries, &ctx);
        assert!(violations.is_empty(), "skimsystem agent can modify .bog files: {violations:?}");
    }

    #[test]
    fn test_skimsystem_agent_disallowed_source_files() {
        let ctx = load_ctx();
        let entries = vec![diff("crates/bog/src/ast.rs")];
        let violations = check_agent_permissions("quality-agent", &entries, &ctx);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.contains("non-.bog file"));
    }

    #[test]
    fn test_unknown_agent_all_violations() {
        let ctx = load_ctx();
        let entries = vec![diff("some/file.rs")];
        let violations = check_agent_permissions("ghost-agent", &entries, &ctx);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.contains("not registered"));
    }
}
