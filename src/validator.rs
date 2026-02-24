use std::collections::HashSet;
use std::path::Path;

use crate::ast::*;
use crate::config::BogbotConfig;
use crate::parser;
use crate::treesitter;

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Parse error in {file}: {message}")]
    Parse { file: String, message: String },

    #[error("In {file}: function '{function}' declared in .bog but not found in source")]
    MissingFunction { file: String, function: String },

    #[error("In {file}: subsystem '{subsystem}' not declared in repo.bog")]
    UndeclaredSubsystem { file: String, subsystem: String },

    #[error("In {file}: owner '{owner}' does not match subsystem '{subsystem}' owner '{expected}'")]
    OwnerMismatch {
        file: String,
        owner: String,
        subsystem: String,
        expected: String,
    },

    #[error("In {file}: file path does not match any glob in subsystem '{subsystem}'")]
    FileNotInSubsystem { file: String, subsystem: String },

    #[error("In {file}: dependency '{dep}' references unknown path")]
    UnknownDependency { file: String, dep: String },

    #[error("Agent '{agent}' not registered in bogbot.toml")]
    UnregisteredAgent { agent: String },
}

#[derive(Debug)]
pub struct ValidationReport {
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
    pub files_checked: usize,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Validate a single .bog file's syntax by parsing it
pub fn validate_syntax(path: &Path) -> Result<BogFile, ValidationError> {
    let content = std::fs::read_to_string(path).map_err(|e| ValidationError::Parse {
        file: path.display().to_string(),
        message: e.to_string(),
    })?;
    parser::parse_bog(&content).map_err(|e| ValidationError::Parse {
        file: path.display().to_string(),
        message: e.to_string(),
    })
}

/// Validate function annotations against the actual source file using tree-sitter
pub fn validate_functions(
    bog_path: &Path,
    bog_file: &BogFile,
    source_path: &Path,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    let source = match std::fs::read_to_string(source_path) {
        Ok(s) => s,
        Err(_) => return errors,
    };

    let symbols = match treesitter::extract_symbols(&source) {
        Ok(s) => s,
        Err(_) => return errors,
    };

    let fn_names: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

    for ann in &bog_file.annotations {
        if let Annotation::Fn(f) = ann {
            if !fn_names.contains(f.name.as_str()) {
                errors.push(ValidationError::MissingFunction {
                    file: bog_path.display().to_string(),
                    function: f.name.clone(),
                });
            }
        }
    }

    errors
}

/// Validate subsystem consistency: file ownership matches repo.bog declarations
pub fn validate_subsystem_consistency(
    repo_bog: &BogFile,
    file_bogs: &[(String, BogFile)],
    config: &BogbotConfig,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Collect subsystem declarations from repo.bog
    let mut subsystems: std::collections::HashMap<String, &SubsystemDecl> =
        std::collections::HashMap::new();
    for ann in &repo_bog.annotations {
        if let Annotation::Subsystem(s) = ann {
            subsystems.insert(s.name.clone(), s);
        }
    }

    // Registered agents
    let registered_agents: HashSet<&str> = config.agents.keys().map(|s| s.as_str()).collect();

    for (path, bog) in file_bogs {
        for ann in &bog.annotations {
            if let Annotation::File(f) = ann {
                // Check subsystem exists
                if !subsystems.contains_key(&f.subsystem) {
                    errors.push(ValidationError::UndeclaredSubsystem {
                        file: path.clone(),
                        subsystem: f.subsystem.clone(),
                    });
                    continue;
                }

                let subsys = subsystems[&f.subsystem];

                // Check owner matches subsystem owner
                if f.owner != subsys.owner {
                    errors.push(ValidationError::OwnerMismatch {
                        file: path.clone(),
                        owner: f.owner.clone(),
                        subsystem: f.subsystem.clone(),
                        expected: subsys.owner.clone(),
                    });
                }

                // Check file path matches at least one glob in subsystem
                let matches_glob = subsys.files.iter().any(|pattern| {
                    glob::Pattern::new(pattern)
                        .map(|p| p.matches(path))
                        .unwrap_or(false)
                });
                if !matches_glob {
                    errors.push(ValidationError::FileNotInSubsystem {
                        file: path.clone(),
                        subsystem: f.subsystem.clone(),
                    });
                }

                // Check agent is registered
                if !registered_agents.contains(f.owner.as_str()) {
                    errors.push(ValidationError::UnregisteredAgent {
                        agent: f.owner.clone(),
                    });
                }
            }
        }
    }

    errors
}

/// Run full validation on a project directory
pub fn validate_project(root: &Path) -> ValidationReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut files_checked = 0;

    // Load config
    let config_path = root.join("bogbot.toml");
    let config = match crate::config::load_config(&config_path) {
        Ok(c) => Some(c),
        Err(e) => {
            warnings.push(format!("Could not load bogbot.toml: {e}"));
            None
        }
    };

    // Parse repo.bog
    let repo_bog_path = root.join("repo.bog");
    let repo_bog = if repo_bog_path.exists() {
        match validate_syntax(&repo_bog_path) {
            Ok(bog) => {
                files_checked += 1;
                Some(bog)
            }
            Err(e) => {
                errors.push(e);
                None
            }
        }
    } else {
        warnings.push("No repo.bog found".to_string());
        None
    };

    // Find and validate all .bog sidecar files
    let mut file_bogs = Vec::new();
    let bog_pattern = root.join("**/*.bog");
    let pattern_str = bog_pattern.to_string_lossy();
    if let Ok(paths) = glob::glob(&pattern_str) {
        for entry in paths.flatten() {
            // Skip repo.bog (already handled)
            if entry.file_name().map(|n| n == "repo.bog").unwrap_or(false) {
                continue;
            }

            match validate_syntax(&entry) {
                Ok(bog) => {
                    files_checked += 1;

                    // If it's a .rs.bog file, validate functions against source
                    let entry_str = entry.to_string_lossy().to_string();
                    if entry_str.ends_with(".rs.bog") {
                        let source_path_str = entry_str.strip_suffix(".bog").unwrap();
                        let source_path = Path::new(source_path_str);
                        if source_path.exists() {
                            let fn_errors = validate_functions(&entry, &bog, source_path);
                            errors.extend(fn_errors);
                        } else {
                            warnings.push(format!(
                                "Source file not found for {entry_str}: expected {source_path_str}"
                            ));
                        }
                    }

                    // Compute relative path for subsystem matching
                    let rel_path = entry
                        .strip_prefix(root)
                        .unwrap_or(&entry)
                        .to_string_lossy()
                        .to_string();
                    // Strip .bog suffix to get the source file relative path
                    let source_rel = rel_path.strip_suffix(".bog").unwrap_or(&rel_path);
                    file_bogs.push((source_rel.to_string(), bog));
                }
                Err(e) => {
                    errors.push(e);
                    files_checked += 1;
                }
            }
        }
    }

    // Subsystem consistency check
    if let (Some(repo), Some(cfg)) = (&repo_bog, &config) {
        let consistency_errors = validate_subsystem_consistency(repo, &file_bogs, cfg);
        errors.extend(consistency_errors);
    }

    ValidationReport {
        errors,
        warnings,
        files_checked,
    }
}
