use std::collections::HashMap;
use std::path::Path;

use colored::Colorize;

use crate::ast::*;
use crate::parser;

#[derive(Debug)]
pub struct SubsystemHealth {
    pub name: String,
    pub owner: String,
    pub status: Status,
    pub file_count: usize,
    pub dimensions: HashMap<String, StatusCount>,
    pub fn_statuses: StatusCount,
}

#[derive(Debug, Default)]
pub struct StatusCount {
    pub green: usize,
    pub yellow: usize,
    pub red: usize,
}

impl StatusCount {
    fn add(&mut self, status: Status) {
        match status {
            Status::Green => self.green += 1,
            Status::Yellow => self.yellow += 1,
            Status::Red => self.red += 1,
        }
    }

    pub fn overall(&self) -> Status {
        if self.red > 0 {
            Status::Red
        } else if self.yellow > 0 {
            Status::Yellow
        } else {
            Status::Green
        }
    }

    pub fn total(&self) -> usize {
        self.green + self.yellow + self.red
    }
}

#[derive(Debug)]
pub struct RepoHealth {
    pub name: String,
    pub subsystems: Vec<SubsystemHealth>,
}

impl RepoHealth {
    pub fn overall_status(&self) -> Status {
        let mut worst = Status::Green;
        for sub in &self.subsystems {
            match sub.status {
                Status::Red => return Status::Red,
                Status::Yellow => worst = Status::Yellow,
                Status::Green => {}
            }
        }
        worst
    }
}

/// Compute health report for the entire project
pub fn compute_health(root: &Path) -> RepoHealth {
    let repo_bog_path = root.join("repo.bog");
    let mut repo_name = "unknown".to_string();
    let mut subsystem_decls: Vec<SubsystemDecl> = Vec::new();

    // Parse repo.bog for subsystem declarations
    if let Ok(content) = std::fs::read_to_string(&repo_bog_path) {
        if let Ok(bog) = parser::parse_bog(&content) {
            for ann in &bog.annotations {
                match ann {
                    Annotation::Repo(r) => repo_name = r.name.clone(),
                    Annotation::Subsystem(s) => subsystem_decls.push(s.clone()),
                    _ => {}
                }
            }
        }
    }

    // Build health for each subsystem
    let mut subsystems = Vec::new();
    for decl in &subsystem_decls {
        let mut sub_health = SubsystemHealth {
            name: decl.name.clone(),
            owner: decl.owner.clone(),
            status: decl.status,
            file_count: 0,
            dimensions: HashMap::new(),
            fn_statuses: StatusCount::default(),
        };

        // Find all .bog files matching this subsystem's globs
        for pattern in &decl.files {
            let full_pattern = root.join(pattern).to_string_lossy().to_string();
            if let Ok(paths) = glob::glob(&full_pattern) {
                for source_path in paths.flatten() {
                    let bog_path_str = format!("{}.bog", source_path.display());
                    let bog_path = Path::new(&bog_path_str);
                    if bog_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(bog_path) {
                            if let Ok(bog) = parser::parse_bog(&content) {
                                sub_health.file_count += 1;
                                aggregate_file_health(&bog, &mut sub_health);
                            }
                        }
                    }
                }
            }
        }

        subsystems.push(sub_health);
    }

    RepoHealth {
        name: repo_name,
        subsystems,
    }
}

fn aggregate_file_health(bog: &BogFile, health: &mut SubsystemHealth) {
    for ann in &bog.annotations {
        match ann {
            Annotation::Health(h) => {
                for (dim, status) in &h.dimensions {
                    health
                        .dimensions
                        .entry(dim.clone())
                        .or_default()
                        .add(*status);
                }
            }
            Annotation::Fn(f) => {
                health.fn_statuses.add(f.status);
            }
            _ => {}
        }
    }
}

/// Format the health report for terminal display
pub fn format_health_report(health: &RepoHealth) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "\n{} {}\n",
        "Project:".bold(),
        health.name.bold()
    ));
    out.push_str(&format!(
        "  Overall: {}\n\n",
        format_status(health.overall_status())
    ));

    if health.subsystems.is_empty() {
        out.push_str("  No subsystems declared.\n");
        return out;
    }

    for sub in &health.subsystems {
        out.push_str(&format!(
            "  {} {} (owner: {})\n",
            format_status(sub.status),
            sub.name.bold(),
            sub.owner
        ));
        out.push_str(&format!("    Files: {}\n", sub.file_count));

        if sub.fn_statuses.total() > 0 {
            out.push_str(&format!(
                "    Functions: {} total ({} green, {} yellow, {} red)\n",
                sub.fn_statuses.total(),
                sub.fn_statuses.green,
                sub.fn_statuses.yellow,
                sub.fn_statuses.red
            ));
        }

        for (dim, counts) in &sub.dimensions {
            out.push_str(&format!(
                "    {}: {}\n",
                dim,
                format_status(counts.overall())
            ));
        }

        out.push('\n');
    }

    out
}

fn format_status(status: Status) -> String {
    match status {
        Status::Green => "●".green().to_string(),
        Status::Yellow => "●".yellow().to_string(),
        Status::Red => "●".red().to_string(),
    }
}
