use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bog::ast::{Annotation, BogFile, SkimsystemDecl, SubsystemDecl};
use bog::config::{AgentRole, BogConfig};

use crate::error::OrchestrateError;

/// Complete loaded context for orchestration decisions.
pub struct RepoContext {
    pub root: PathBuf,
    pub config: BogConfig,
    pub repo_bog: BogFile,
    pub repo_bog_raw: String,
    pub subsystems: HashMap<String, SubsystemDecl>,
    pub skimsystems: HashMap<String, SkimsystemDecl>,
    /// Maps agent name → list of subsystem names they own.
    pub agent_to_subsystems: HashMap<String, Vec<String>>,
    /// Maps agent name → list of skimsystem names they own.
    pub agent_to_skimsystems: HashMap<String, Vec<String>>,
}

impl RepoContext {
    /// Load repo context from a project root directory.
    pub fn load(root: &Path) -> Result<Self, OrchestrateError> {
        let config_path = root.join("bog.toml");
        let config = bog::config::load_config(&config_path)
            .map_err(|e| OrchestrateError::ContextLoad(format!("bog.toml: {e}")))?;

        let repo_bog_path = root.join("repo.bog");
        let repo_bog_raw = std::fs::read_to_string(&repo_bog_path)
            .map_err(|e| OrchestrateError::ContextLoad(format!("repo.bog: {e}")))?;
        let repo_bog = bog::parser::parse_bog(&repo_bog_raw)
            .map_err(|e| OrchestrateError::ContextLoad(format!("repo.bog parse: {e}")))?;

        let mut subsystems = HashMap::new();
        let mut skimsystems = HashMap::new();
        let mut agent_to_subsystems: HashMap<String, Vec<String>> = HashMap::new();
        let mut agent_to_skimsystems: HashMap<String, Vec<String>> = HashMap::new();

        for ann in &repo_bog.annotations {
            match ann {
                Annotation::Subsystem(s) => {
                    agent_to_subsystems
                        .entry(s.owner.clone())
                        .or_default()
                        .push(s.name.clone());
                    subsystems.insert(s.name.clone(), s.clone());
                }
                Annotation::Skimsystem(s) => {
                    agent_to_skimsystems
                        .entry(s.owner.clone())
                        .or_default()
                        .push(s.name.clone());
                    skimsystems.insert(s.name.clone(), s.clone());
                }
                _ => {}
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            config,
            repo_bog,
            repo_bog_raw,
            subsystems,
            skimsystems,
            agent_to_subsystems,
            agent_to_skimsystems,
        })
    }

    /// Get all file glob patterns owned by a given agent (union of all their subsystems).
    pub fn agent_file_globs(&self, agent_name: &str) -> Vec<String> {
        let Some(sub_names) = self.agent_to_subsystems.get(agent_name) else {
            return Vec::new();
        };
        sub_names
            .iter()
            .flat_map(|name| {
                self.subsystems
                    .get(name)
                    .map(|s| s.files.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Get the agent role for a given agent name.
    pub fn agent_role(&self, agent_name: &str) -> Option<AgentRole> {
        self.config.agents.get(agent_name).map(|a| a.role)
    }

    /// Format the agent registry for embedding in prompts.
    pub fn format_agent_registry(&self) -> String {
        let mut lines = Vec::new();
        for (name, agent) in &self.config.agents {
            lines.push(format!(
                "- {name} (role: {role:?}): {desc}",
                role = agent.role,
                desc = agent.description
            ));
        }
        lines.join("\n")
    }

    /// Format subsystem ownership summary for prompts.
    pub fn format_subsystem_summary(&self) -> String {
        let mut lines = Vec::new();
        for (name, sub) in &self.subsystems {
            lines.push(format!(
                "- {name} (owner: {owner}): files = {files:?}",
                owner = sub.owner,
                files = sub.files
            ));
        }
        lines.join("\n")
    }

    /// Format skimsystem summary for prompts.
    pub fn format_skimsystem_summary(&self) -> String {
        let mut lines = Vec::new();
        for (name, skim) in &self.skimsystems {
            lines.push(format!(
                "- {name} (owner: {owner}): targets = {targets:?}",
                owner = skim.owner,
                targets = skim.targets
            ));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn test_load_context() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        assert_eq!(ctx.config.bog.version, "0.1.0");
        assert!(ctx.subsystems.contains_key("core"));
        assert!(ctx.subsystems.contains_key("analysis"));
        assert!(ctx.subsystems.contains_key("cli"));
        assert!(ctx.skimsystems.contains_key("annotation-quality"));
        assert!(ctx.skimsystems.contains_key("code-quality"));
    }

    #[test]
    fn test_agent_file_globs() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        let globs = ctx.agent_file_globs("core-agent");
        assert!(globs.iter().any(|g| g.contains("ast.rs")));
        assert!(globs.iter().any(|g| g.contains("parser.rs")));
    }

    #[test]
    fn test_agent_role() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        assert_eq!(ctx.agent_role("core-agent"), Some(AgentRole::Subsystem));
        assert_eq!(
            ctx.agent_role("code-standards-agent"),
            Some(AgentRole::Skimsystem)
        );
        assert_eq!(
            ctx.agent_role("observability-agent"),
            Some(AgentRole::Skimsystem)
        );
        assert_eq!(ctx.agent_role("nonexistent"), None);
    }

    #[test]
    fn test_agent_to_subsystems_mapping() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        let subs = ctx.agent_to_subsystems.get("analysis-agent").unwrap();
        assert!(subs.contains(&"analysis".to_string()));
        assert!(subs.contains(&"test-fixtures".to_string()));
    }
}
