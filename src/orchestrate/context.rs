use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{
    self, Annotation, BogFile, DerivedAgents, SkimTargets, SkimsystemDecl, SubsystemDecl,
};
use crate::config::{AgentRole, BogConfig};

use super::error::OrchestrateError;

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
    /// Agent registry derived from repo.bog declarations.
    pub derived_agents: DerivedAgents,
    /// Parsed sidecar .bog files keyed by relative source path (e.g. "src/ast.rs").
    pub sidecar_bogs: HashMap<String, BogFile>,
}

impl RepoContext {
    /// Load repo context from a project root directory.
    pub fn load(root: &Path) -> Result<Self, OrchestrateError> {
        let config_path = root.join("bog.toml");
        let config = crate::config::load_config(&config_path)
            .map_err(|e| OrchestrateError::ContextLoad(format!("bog.toml: {e}")))?;

        let repo_bog_path = root.join("repo.bog");
        let repo_bog_raw = std::fs::read_to_string(&repo_bog_path)
            .map_err(|e| OrchestrateError::ContextLoad(format!("repo.bog: {e}")))?;
        let repo_bog = crate::parser::parse_bog(&repo_bog_raw)
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

        let derived_agents = ast::derive_agents(&repo_bog);
        let sidecar_bogs = load_all_sidecars(root, &subsystems);

        Ok(Self {
            root: root.to_path_buf(),
            config,
            repo_bog,
            repo_bog_raw,
            subsystems,
            skimsystems,
            agent_to_subsystems,
            agent_to_skimsystems,
            derived_agents,
            sidecar_bogs,
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
        self.derived_agents.roles.get(agent_name).copied()
    }

    /// Format the agent registry for embedding in prompts.
    pub fn format_agent_registry(&self) -> String {
        let mut lines: Vec<String> = self.derived_agents.roles.iter().map(|(name, role)| {
            let desc = self.derived_agents.descriptions.get(name)
                .map(|s| s.as_str())
                .unwrap_or("(no description)");
            format!("- {name} (role: {role:?}): {desc}")
        }).collect();
        lines.sort();
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

    /// Get sidecar .bog files matching a set of file paths.
    pub fn sidecar_bogs_for_files(&self, files: &[String]) -> Vec<(String, &BogFile)> {
        let mut result = Vec::new();
        for file_path in files {
            if let Some(bog) = self.sidecar_bogs.get(file_path.as_str()) {
                result.push((file_path.clone(), bog));
            }
        }
        result.sort_by_key(|(path, _)| path.clone());
        result
    }

    /// Get sidecar .bog files for all files owned by an agent.
    pub fn agent_sidecar_bogs(&self, agent_name: &str) -> Vec<(String, &BogFile)> {
        let globs = self.agent_file_globs(agent_name);
        self.sidecar_bogs_for_files(&globs)
    }

    /// Get sidecar .bog files within a skimsystem's target scope.
    pub fn skimsystem_sidecar_bogs(&self, skim_name: &str) -> Vec<(String, &BogFile)> {
        let Some(skim) = self.skimsystems.get(skim_name) else {
            return Vec::new();
        };
        match &skim.targets {
            SkimTargets::All => {
                let mut result: Vec<(String, &BogFile)> = self
                    .sidecar_bogs
                    .iter()
                    .map(|(k, v)| (k.clone(), v))
                    .collect();
                result.sort_by_key(|(path, _)| path.clone());
                result
            }
            SkimTargets::Named(subsystem_names) => {
                let mut files = Vec::new();
                for sub_name in subsystem_names {
                    if let Some(sub) = self.subsystems.get(sub_name) {
                        files.extend(sub.files.clone());
                    }
                }
                self.sidecar_bogs_for_files(&files)
            }
        }
    }
}

/// Load all sidecar .bog files for files declared in subsystems.
fn load_all_sidecars(
    root: &Path,
    subsystems: &HashMap<String, SubsystemDecl>,
) -> HashMap<String, BogFile> {
    let mut sidecars = HashMap::new();
    for sub in subsystems.values() {
        for file_path in &sub.files {
            let bog_path = root.join(format!("{file_path}.bog"));
            let Ok(content) = std::fs::read_to_string(&bog_path) else {
                continue;
            };
            let Ok(bog) = crate::parser::parse_bog(&content) else {
                continue;
            };
            sidecars.insert(file_path.clone(), bog);
        }
    }
    sidecars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
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

    #[test]
    fn test_sidecar_bogs_loaded() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        assert!(
            !ctx.sidecar_bogs.is_empty(),
            "Should load at least some sidecar .bog files"
        );
        // Core files should have sidecars
        assert!(ctx.sidecar_bogs.contains_key("src/ast.rs"));
        assert!(ctx.sidecar_bogs.contains_key("src/parser.rs"));
    }

    #[test]
    fn test_agent_sidecar_bogs_scoped() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        let core_sidecars = ctx.agent_sidecar_bogs("core-agent");
        let core_paths: Vec<&str> = core_sidecars.iter().map(|(p, _)| p.as_str()).collect();
        assert!(core_paths.contains(&"src/ast.rs"));
        assert!(core_paths.contains(&"src/parser.rs"));
        // Should NOT contain cli files
        assert!(!core_paths.contains(&"src/cli.rs"));
    }

    #[test]
    fn test_skimsystem_sidecar_bogs_all() {
        let root = workspace_root();
        let ctx = RepoContext::load(&root).unwrap();
        // code-quality targets all — should get all sidecars
        let all_sidecars = ctx.skimsystem_sidecar_bogs("code-quality");
        assert!(
            all_sidecars.len() >= 4,
            "Should return sidecars from all subsystems"
        );
    }
}
