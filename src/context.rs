use std::collections::HashMap;
use std::path::{Path, PathBuf};

use colored::Colorize;
use serde::Serialize;

use crate::ast::{
    self, Annotation, BogFile, SkimTarget, SubsystemDecl, Value,
};
use crate::parser;

// --- Error type ---

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("Agent '{0}' not declared as owner in repo.bog")]
    UnknownAgent(String),
    #[error("Subsystem '{0}' not declared in repo.bog")]
    UnknownSubsystem(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse repo.bog")]
    RepoBogParse,
}

// --- Scoping & filtering ---

pub enum ContextScope {
    All,
    Agent(String),
    Subsystem(String),
}

pub struct SectionFilter {
    pub pickled: bool,
    pub requests: bool,
    pub health: bool,
    pub contracts: bool,
    pub skims: bool,
}

impl SectionFilter {
    pub fn all() -> Self {
        Self {
            pickled: true,
            requests: true,
            health: true,
            contracts: true,
            skims: true,
        }
    }
}

// --- Output types ---

#[derive(Debug, Serialize)]
pub struct ContextOutput {
    pub scope: ScopeInfo,
    pub files: Vec<FileContext>,
}

#[derive(Debug, Serialize)]
pub struct ScopeInfo {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub subsystems: Vec<SubsystemInfo>,
}

#[derive(Debug, Serialize)]
pub struct SubsystemInfo {
    pub name: String,
    pub owner: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct FileContext {
    pub path: String,
    pub subsystem: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<HealthOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pickled: Vec<PickledOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub change_requests: Vec<ChangeRequestOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fn_contracts: Vec<FnContractOutput>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skim_observations: Vec<SkimObservationOutput>,
}

#[derive(Debug, Serialize)]
pub struct HealthOutput {
    pub dimensions: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct PickledOutput {
    pub id: String,
    pub agent: String,
    pub updated: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    pub tags: Vec<String>,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ChangeRequestOutput {
    pub id: String,
    pub from: String,
    pub target: String,
    pub change_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    pub created: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct FnContractOutput {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractOutput>,
    pub deps: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ContractOutput {
    pub inputs: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub invariants: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkimObservationOutput {
    pub skimsystem: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

// --- Loading ---

/// Load annotation context from .bog files, scoped to an agent, subsystem, or all.
pub fn load_context(
    root: &Path,
    scope: ContextScope,
    filter: SectionFilter,
    kind_filter: Option<&str>,
    tag_filter: Option<&str>,
) -> Result<ContextOutput, ContextError> {
    // 1. Parse repo.bog
    let repo_bog_path = root.join("repo.bog");
    let repo_content = std::fs::read_to_string(&repo_bog_path)?;
    let repo_bog = parser::parse_bog(&repo_content).map_err(|_| ContextError::RepoBogParse)?;

    let subsystem_decls: Vec<SubsystemDecl> = repo_bog
        .annotations
        .iter()
        .filter_map(|a| {
            if let Annotation::Subsystem(s) = a {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    // 2. Resolve scope
    let (scope_info, target_subsystems) = resolve_scope(&scope, &subsystem_decls, &repo_bog)?;

    // 4. Discover and parse .bog sidecars within scope
    let mut files = Vec::new();

    for decl in &target_subsystems {
        for pattern in &decl.files {
            let full_pattern = root.join(pattern).to_string_lossy().to_string();
            let Ok(paths) = glob::glob(&full_pattern) else {
                continue;
            };
            for source_path in paths.flatten() {
                let bog_path = PathBuf::from(format!("{}.bog", source_path.display()));
                if !bog_path.exists() {
                    continue;
                }

                let content = match std::fs::read_to_string(&bog_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let bog = match parser::parse_bog(&content) {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                let rel_path = source_path
                    .strip_prefix(root)
                    .unwrap_or(&source_path)
                    .to_string_lossy()
                    .to_string();

                let file_ctx =
                    extract_file_context(&rel_path, &decl.name, &bog, &filter, kind_filter, tag_filter);
                files.push(file_ctx);
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(ContextOutput {
        scope: scope_info,
        files,
    })
}

fn resolve_scope(
    scope: &ContextScope,
    subsystem_decls: &[SubsystemDecl],
    repo_bog: &BogFile,
) -> Result<(ScopeInfo, Vec<SubsystemDecl>), ContextError> {
    match scope {
        ContextScope::All => {
            let scope_info = ScopeInfo {
                kind: "all".to_string(),
                name: None,
                subsystems: subsystem_decls.iter().map(decl_to_info).collect(),
            };
            Ok((scope_info, subsystem_decls.to_vec()))
        }
        ContextScope::Agent(agent_name) => {
            let derived = ast::derive_agents(repo_bog);
            if !derived.roles.contains_key(agent_name) {
                return Err(ContextError::UnknownAgent(agent_name.clone()));
            }
            let matching: Vec<SubsystemDecl> = subsystem_decls
                .iter()
                .filter(|s| s.owner == *agent_name)
                .cloned()
                .collect();
            let scope_info = ScopeInfo {
                kind: "agent".to_string(),
                name: Some(agent_name.clone()),
                subsystems: matching.iter().map(decl_to_info).collect(),
            };
            Ok((scope_info, matching))
        }
        ContextScope::Subsystem(sub_name) => {
            let matching: Vec<SubsystemDecl> = subsystem_decls
                .iter()
                .filter(|s| s.name == *sub_name)
                .cloned()
                .collect();
            if matching.is_empty() {
                return Err(ContextError::UnknownSubsystem(sub_name.clone()));
            }
            let scope_info = ScopeInfo {
                kind: "subsystem".to_string(),
                name: Some(sub_name.clone()),
                subsystems: matching.iter().map(decl_to_info).collect(),
            };
            Ok((scope_info, matching))
        }
    }
}

fn decl_to_info(decl: &SubsystemDecl) -> SubsystemInfo {
    SubsystemInfo {
        name: decl.name.clone(),
        owner: decl.owner.clone(),
        status: decl.status.to_string(),
    }
}

fn extract_file_context(
    rel_path: &str,
    subsystem: &str,
    bog: &BogFile,
    filter: &SectionFilter,
    kind_filter: Option<&str>,
    tag_filter: Option<&str>,
) -> FileContext {
    let mut owner = None;
    let mut status = None;
    let mut description = None;
    let mut health = None;
    let mut pickled = Vec::new();
    let mut change_requests = Vec::new();
    let mut fn_contracts = Vec::new();
    let mut skim_observations = Vec::new();

    for ann in &bog.annotations {
        match ann {
            Annotation::File(f) => {
                owner = Some(f.owner.clone());
                status = Some(f.status.to_string());
            }
            Annotation::Description(d) => {
                description = Some(d.clone());
            }
            Annotation::Health(h) if filter.health => {
                health = Some(HealthOutput {
                    dimensions: h
                        .dimensions
                        .iter()
                        .map(|(k, v)| (k.clone(), v.to_string()))
                        .collect(),
                });
            }
            Annotation::Pickled(p) if filter.pickled => {
                if let Some(kind) = kind_filter
                    && p.kind.to_string() != kind
                {
                    continue;
                }
                if let Some(tag) = tag_filter
                    && !p.tags.iter().any(|t| t.to_string() == tag)
                {
                    continue;
                }
                pickled.push(PickledOutput {
                    id: p.id.clone(),
                    agent: p.agent.clone(),
                    updated: p.updated.clone(),
                    kind: p.kind.to_string(),
                    supersedes: p.supersedes.clone(),
                    tags: p.tags.iter().map(|t| t.to_string()).collect(),
                    content: p.content.clone(),
                });
            }
            Annotation::ChangeRequests(reqs) if filter.requests => {
                for r in reqs {
                    change_requests.push(ChangeRequestOutput {
                        id: r.id.clone(),
                        from: r.from.clone(),
                        target: format_target(&r.target),
                        change_type: r.change_type.clone(),
                        status: r.status.clone(),
                        priority: r.priority.clone(),
                        created: r.created.clone(),
                        description: r.description.clone(),
                    });
                }
            }
            Annotation::Fn(f) if filter.contracts => {
                fn_contracts.push(FnContractOutput {
                    name: f.name.clone(),
                    status: f.status.to_string(),
                    description: f.description.clone(),
                    contract: f.contract.as_ref().map(|c| ContractOutput {
                        inputs: c.inputs.clone(),
                        output: c.output.clone(),
                        invariants: c.invariants.clone(),
                    }),
                    deps: f.deps.clone(),
                });
            }
            Annotation::Skim(obs) if filter.skims => {
                skim_observations.push(SkimObservationOutput {
                    skimsystem: obs.skimsystem.clone(),
                    status: obs.status.to_string(),
                    notes: obs.notes.clone(),
                    target: obs.target.as_ref().map(format_skim_target),
                });
            }
            _ => {}
        }
    }

    FileContext {
        path: rel_path.to_string(),
        subsystem: subsystem.to_string(),
        owner,
        status,
        description,
        health,
        pickled,
        change_requests,
        fn_contracts,
        skim_observations,
    }
}

fn format_target(v: &Value) -> String {
    match v {
        Value::Ident(s) | Value::String(s) => s.clone(),
        Value::FnRef(name) => format!("fn({name})"),
        Value::Path(parts) => parts.join("::"),
        _ => format!("{v:?}"),
    }
}

fn format_skim_target(t: &SkimTarget) -> String {
    match t {
        SkimTarget::File => "file".to_string(),
        SkimTarget::Fn(name) => format!("fn({name})"),
    }
}

// --- Text formatting ---

fn format_status_dot(status_str: &str) -> String {
    match status_str {
        "green" => "●".green().to_string(),
        "yellow" => "●".yellow().to_string(),
        "red" => "●".red().to_string(),
        _ => "●".dimmed().to_string(),
    }
}

/// Format context output as colored, sectioned terminal text.
pub fn format_context_text(output: &ContextOutput) -> String {
    let mut out = String::new();

    if output.files.is_empty() {
        out.push_str("  No annotated files in scope.\n");
        return out;
    }

    // Group files by subsystem
    let mut by_subsystem: HashMap<String, Vec<&FileContext>> = HashMap::new();
    for file in &output.files {
        by_subsystem
            .entry(file.subsystem.clone())
            .or_default()
            .push(file);
    }

    for sub_info in &output.scope.subsystems {
        let dot = format_status_dot(&sub_info.status);
        out.push_str(&format!(
            "\n{} {} (owner: {}, {dot})\n",
            "═══".bold(),
            sub_info.name.bold(),
            sub_info.owner,
        ));

        let Some(files) = by_subsystem.get(&sub_info.name) else {
            continue;
        };

        for file in files {
            let file_name = file.path.rsplit('/').next().unwrap_or(&file.path);
            out.push_str(&format!("\n  {}\n", file_name.bold()));

            format_health_section(&mut out, file);
            format_pickled_section(&mut out, file);
            format_requests_section(&mut out, file);
            format_contracts_section(&mut out, file);
            format_skims_section(&mut out, file);
        }

        out.push('\n');
    }

    out
}

fn format_health_section(out: &mut String, file: &FileContext) {
    let Some(h) = &file.health else { return };
    let dims: Vec<String> = h
        .dimensions
        .iter()
        .map(|(k, v)| format!("{k}: {}", format_status_dot(v)))
        .collect();
    if !dims.is_empty() {
        out.push_str(&format!("    {} {}\n", "[health]".dimmed(), dims.join(", ")));
    }
}

fn format_pickled_section(out: &mut String, file: &FileContext) {
    if file.pickled.is_empty() {
        return;
    }
    out.push_str(&format!(
        "    {} ({})\n",
        "[pickled]".dimmed(),
        file.pickled.len()
    ));
    for p in &file.pickled {
        let supersedes_str = p
            .supersedes
            .as_ref()
            .map(|s| format!(" supersedes:{s}"))
            .unwrap_or_default();
        let tags_str = if p.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", p.tags.join(", "))
        };
        out.push_str(&format!(
            "      {} · {}{tags_str}{supersedes_str} · {}\n",
            p.id, p.kind, p.updated
        ));
        let preview = if p.content.len() > 120 {
            format!("{}...", &p.content[..120])
        } else {
            p.content.clone()
        };
        out.push_str(&format!("        \"{preview}\"\n"));
    }
}

fn format_requests_section(out: &mut String, file: &FileContext) {
    if file.change_requests.is_empty() {
        return;
    }
    let pending = file
        .change_requests
        .iter()
        .filter(|r| r.status == "pending")
        .count();
    out.push_str(&format!(
        "    {} ({pending} pending)\n",
        "[requests]".dimmed(),
    ));
    for r in &file.change_requests {
        let status_color = match r.status.as_str() {
            "pending" => r.status.yellow().to_string(),
            "resolved" => r.status.green().to_string(),
            "denied" => r.status.red().to_string(),
            _ => r.status.clone(),
        };
        out.push_str(&format!(
            "      {} · {} → {} · {status_color}\n",
            r.id, r.from, r.target
        ));
        out.push_str(&format!("        \"{}\"\n", r.description));
    }
}

fn format_contracts_section(out: &mut String, file: &FileContext) {
    if file.fn_contracts.is_empty() {
        return;
    }
    out.push_str(&format!(
        "    {} ({})\n",
        "[fn contracts]".dimmed(),
        file.fn_contracts.len()
    ));
    for f in &file.fn_contracts {
        let dot = format_status_dot(&f.status);
        out.push_str(&format!("      {} — {dot}\n", f.name));
        if let Some(c) = &f.contract {
            if !c.inputs.is_empty() {
                let inputs: Vec<String> = c
                    .inputs
                    .iter()
                    .map(|(name, ty)| format!("{name}: {ty}"))
                    .collect();
                out.push_str(&format!("        in: ({})", inputs.join(", ")));
                if let Some(ret) = &c.output {
                    out.push_str(&format!(" → {ret}"));
                }
                out.push('\n');
            } else if let Some(ret) = &c.output {
                out.push_str(&format!("        out: {ret}\n"));
            }
        }
        if !f.deps.is_empty() {
            out.push_str(&format!("        deps: {}\n", f.deps.join(", ")));
        }
    }
}

fn format_skims_section(out: &mut String, file: &FileContext) {
    if file.skim_observations.is_empty() {
        return;
    }
    out.push_str(&format!(
        "    {} ({})\n",
        "[skims]".dimmed(),
        file.skim_observations.len()
    ));
    for obs in &file.skim_observations {
        let dot = format_status_dot(&obs.status);
        let target_str = obs
            .target
            .as_ref()
            .map(|t| format!(" {t}"))
            .unwrap_or_default();
        out.push_str(&format!("      {dot}  {}{target_str}", obs.skimsystem));
        if let Some(notes) = &obs.notes {
            out.push_str(&format!("  \"{notes}\""));
        }
        out.push('\n');
    }
}

/// Discover all .bog sidecar files under root, skipping target/, .git/, and repo.bog.
pub fn discover_bog_files(root: &Path) -> Vec<PathBuf> {
    let pattern = root.join("**/*.bog");
    let Ok(paths) = glob::glob(&pattern.to_string_lossy()) else {
        return Vec::new();
    };

    paths
        .flatten()
        .filter(|p| {
            let rel = p.strip_prefix(root).unwrap_or(p);
            !rel.components().any(|c| {
                matches!(c.as_os_str().to_str(), Some("target" | ".git"))
            }) && !p
                .file_name()
                .map(|n| n == "repo.bog")
                .unwrap_or(false)
        })
        .collect()
}
