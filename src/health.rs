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
pub struct SkimsystemHealth {
    pub name: String,
    pub owner: String,
    pub status: Status,
    pub observation_count: usize,
    pub observation_statuses: StatusCount,
    pub subsystem_coverage: Vec<(String, StatusCount)>,
}

#[derive(Debug)]
pub struct RepoHealth {
    pub name: String,
    pub subsystems: Vec<SubsystemHealth>,
    pub skimsystems: Vec<SkimsystemHealth>,
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
        for sk in &self.skimsystems {
            match sk.status {
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
    let mut skimsystem_decls: Vec<SkimsystemDecl> = Vec::new();

    // Parse repo.bog for subsystem and skimsystem declarations
    if let Ok(content) = std::fs::read_to_string(&repo_bog_path) {
        if let Ok(bog) = parser::parse_bog(&content) {
            for ann in &bog.annotations {
                match ann {
                    Annotation::Repo(r) => repo_name = r.name.clone(),
                    Annotation::Subsystem(s) => subsystem_decls.push(s.clone()),
                    Annotation::Skimsystem(sk) => skimsystem_decls.push(sk.clone()),
                    _ => {}
                }
            }
        }
    }

    // Map subsystem names to their file globs for skim observation matching
    let subsystem_map: HashMap<String, &SubsystemDecl> = subsystem_decls
        .iter()
        .map(|s| (s.name.clone(), s))
        .collect();

    // Build health for each subsystem, collecting parsed bogs for skim aggregation
    let mut subsystems = Vec::new();
    let mut all_file_bogs: Vec<(String, BogFile)> = Vec::new(); // (subsystem_name, bog)

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
                                all_file_bogs.push((decl.name.clone(), bog));
                            }
                        }
                    }
                }
            }
        }

        subsystems.push(sub_health);
    }

    // Build health for each skimsystem
    let mut skimsystems = Vec::new();
    for sk_decl in &skimsystem_decls {
        let mut sk_health = SkimsystemHealth {
            name: sk_decl.name.clone(),
            owner: sk_decl.owner.clone(),
            status: sk_decl.status,
            observation_count: 0,
            observation_statuses: StatusCount::default(),
            subsystem_coverage: Vec::new(),
        };

        // Determine which subsystems this skimsystem targets
        let target_subsystems: Vec<&str> = match &sk_decl.targets {
            SkimTargets::All => subsystem_map.keys().map(|s| s.as_str()).collect(),
            SkimTargets::Named(names) => names.iter().map(|s| s.as_str()).collect(),
        };

        // Aggregate skim observations per targeted subsystem
        let mut per_subsystem: HashMap<String, StatusCount> = HashMap::new();
        for target in &target_subsystems {
            per_subsystem.insert(target.to_string(), StatusCount::default());
        }

        for (subsystem_name, bog) in &all_file_bogs {
            if !target_subsystems.contains(&subsystem_name.as_str()) {
                continue;
            }
            for ann in &bog.annotations {
                if let Annotation::Skim(obs) = ann {
                    if obs.skimsystem == sk_decl.name {
                        sk_health.observation_count += 1;
                        sk_health.observation_statuses.add(obs.status);
                        if let Some(counts) = per_subsystem.get_mut(subsystem_name) {
                            counts.add(obs.status);
                        }
                    }
                }
            }
        }

        sk_health.subsystem_coverage = per_subsystem.into_iter().collect();
        sk_health.subsystem_coverage.sort_by(|a, b| a.0.cmp(&b.0));

        skimsystems.push(sk_health);
    }

    RepoHealth {
        name: repo_name,
        subsystems,
        skimsystems,
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

    // Skimsystem health
    if !health.skimsystems.is_empty() {
        out.push_str(&format!("  {}\n\n", "Skimsystems:".bold()));

        for sk in &health.skimsystems {
            out.push_str(&format!(
                "  {} {} (owner: {})\n",
                format_status(sk.observation_statuses.overall()),
                sk.name.bold(),
                sk.owner
            ));

            if sk.observation_count > 0 {
                out.push_str(&format!(
                    "    Observations: {} total ({} green, {} yellow, {} red)\n",
                    sk.observation_count,
                    sk.observation_statuses.green,
                    sk.observation_statuses.yellow,
                    sk.observation_statuses.red
                ));
            } else {
                out.push_str("    Observations: none\n");
            }

            for (subsys, counts) in &sk.subsystem_coverage {
                if counts.total() > 0 {
                    out.push_str(&format!(
                        "    {}: {} ({} observations)\n",
                        subsys,
                        format_status(counts.overall()),
                        counts.total()
                    ));
                }
            }

            out.push('\n');
        }
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
