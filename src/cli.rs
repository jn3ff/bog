use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::health;
use crate::stub;
use crate::validator;

#[derive(Parser)]
#[command(name = "bogbot", version, about = "Agent-first codebase annotation system")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize bogbot in the current directory
    Init,

    /// Validate .bog files (syntax + tree-sitter)
    Validate {
        /// Path to validate (defaults to current directory)
        path: Option<PathBuf>,
    },

    /// Show health status for all subsystems
    Status {
        /// Path to project root (defaults to current directory)
        path: Option<PathBuf>,
    },

    /// Check subsystem/file ownership consistency
    Check {
        /// Path to project root (defaults to current directory)
        path: Option<PathBuf>,
    },

    /// Show skimsystem health and observations
    Skim {
        /// Path to project root (defaults to current directory)
        path: Option<PathBuf>,

        /// Show only a specific skimsystem
        #[arg(long)]
        name: Option<String>,

        /// Show individual observations
        #[arg(short, long)]
        verbose: bool,
    },

    /// Generate stub annotations for unannotated functions
    Stub {
        /// Path to process (defaults to current directory)
        path: Option<PathBuf>,

        /// List all current stub annotations instead of generating new ones
        #[arg(long)]
        list: bool,
    },
}

pub fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Init => cmd_init(),
        Command::Validate { path } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            cmd_validate(&root)
        }
        Command::Status { path } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            cmd_status(&root)
        }
        Command::Check { path } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            cmd_check(&root)
        }
        Command::Skim { path, name, verbose } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            cmd_skim(&root, name.as_deref(), verbose)
        }
        Command::Stub { path, list } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            if list {
                cmd_stub_list(&root)
            } else {
                cmd_stub(&root)
            }
        }
    }
}

fn cmd_init() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(".");

    // Create bogbot.toml
    let config_path = root.join("bogbot.toml");
    if config_path.exists() {
        println!("{} bogbot.toml already exists, skipping", "note:".yellow());
    } else {
        std::fs::write(
            &config_path,
            r#"[bogbot]
version = "0.1.0"

[agents]
# my-agent = { role = "subsystem", description = "Owns some subsystem" }
# my-skim-agent = { role = "skimsystem", description = "Cross-cutting quality observer" }

[tree_sitter]
language = "rust"

[health]
dimensions = ["test_coverage", "staleness", "complexity", "contract_compliance"]
"#,
        )?;
        println!("{} created bogbot.toml", "ok:".green());
    }

    // Create repo.bog
    let repo_bog_path = root.join("repo.bog");
    if repo_bog_path.exists() {
        println!("{} repo.bog already exists, skipping", "note:".yellow());
    } else {
        std::fs::write(
            &repo_bog_path,
            r#"#[repo(
  name = "my-project",
  version = "0.1.0",
  updated = "2026-01-01"
)]

#[description {
  Describe your project here.
}]

// Declare subsystems:
// #[subsystem(example) {
//   owner = "my-agent",
//   files = ["src/example/*.rs"],
//   status = green,
//   description = "Example subsystem"
// }]
"#,
        )?;
        println!("{} created repo.bog", "ok:".green());
    }

    // Create an example sidecar if src/main.rs exists
    let example_src = root.join("src/main.rs");
    let example_bog = root.join("src/main.rs.bog");
    if example_src.exists() && !example_bog.exists() {
        std::fs::write(
            &example_bog,
            r#"#[file(
  owner = "my-agent",
  subsystem = "core",
  updated = "2026-01-01",
  status = green
)]

#[description {
  Main entry point.
}]

#[health(
  test_coverage = yellow,
  staleness = green
)]
"#,
        )?;
        println!("{} created src/main.rs.bog (example)", "ok:".green());
    }

    println!(
        "\n{} Edit bogbot.toml to register agents and configure health dimensions.",
        "next:".bold()
    );
    println!(
        "      Edit repo.bog to declare subsystems.\n      Run {} to verify.",
        "bogbot validate".bold()
    );

    Ok(())
}

fn cmd_validate(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Validating .bog files...".bold());
    let report = validator::validate_project(root);

    for warning in &report.warnings {
        println!("  {} {warning}", "warn:".yellow());
    }

    for error in &report.errors {
        println!("  {} {error}", "error:".red());
    }

    println!(
        "\n  Files checked: {}",
        report.files_checked.to_string().bold()
    );

    if report.is_ok() {
        println!("  {}", "All checks passed.".green().bold());
        Ok(())
    } else {
        println!(
            "  {} {} error(s) found.",
            "FAIL:".red().bold(),
            report.errors.len()
        );
        std::process::exit(1);
    }
}

fn cmd_status(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let repo_health = health::compute_health(root);
    let report = health::format_health_report(&repo_health);
    print!("{report}");

    // Skimsystem summary if not already shown by format_health_report having entries
    if repo_health.skimsystems.is_empty() {
        return Ok(());
    }

    println!(
        "  Run {} for skimsystem details.\n",
        "bogbot skim".bold()
    );

    Ok(())
}

fn cmd_check(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Checking subsystem consistency...".bold());
    let report = validator::validate_project(root);

    // Filter to only subsystem/ownership errors
    let ownership_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| {
            matches!(
                e,
                validator::ValidationError::UndeclaredSubsystem { .. }
                    | validator::ValidationError::OwnerMismatch { .. }
                    | validator::ValidationError::FileNotInSubsystem { .. }
                    | validator::ValidationError::UnregisteredAgent { .. }
                    | validator::ValidationError::UndeclaredSkimsystem { .. }
                    | validator::ValidationError::SkimsystemTargetNotFound { .. }
                    | validator::ValidationError::UnregisteredSkimAgent { .. }
                    | validator::ValidationError::SkimTargetFunctionMissing { .. }
                    | validator::ValidationError::AgentRoleMismatch { .. }
            )
        })
        .collect();

    for error in &ownership_errors {
        println!("  {} {error}", "error:".red());
    }

    if ownership_errors.is_empty() {
        println!("  {}", "Ownership consistency checks passed.".green().bold());
        Ok(())
    } else {
        println!(
            "  {} {} error(s) found.",
            "FAIL:".red().bold(),
            ownership_errors.len()
        );
        std::process::exit(1);
    }
}

fn cmd_skim(
    root: &Path,
    name_filter: Option<&str>,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let repo_health = health::compute_health(root);

    if repo_health.skimsystems.is_empty() {
        println!("{}", "No skimsystems declared in repo.bog.".yellow());
        return Ok(());
    }

    let filtered: Vec<_> = repo_health
        .skimsystems
        .iter()
        .filter(|sk| name_filter.is_none() || name_filter == Some(sk.name.as_str()))
        .collect();

    if filtered.is_empty() {
        println!(
            "  {} skimsystem '{}' not found.",
            "error:".red(),
            name_filter.unwrap_or("?")
        );
        std::process::exit(1);
    }

    // Load principles from repo.bog for display
    let repo_bog_path = root.join("repo.bog");
    let skimsystem_decls: Vec<crate::ast::SkimsystemDecl> =
        if let Ok(content) = std::fs::read_to_string(&repo_bog_path) {
            if let Ok(bog) = crate::parser::parse_bog(&content) {
                bog.annotations
                    .into_iter()
                    .filter_map(|a| {
                        if let crate::ast::Annotation::Skimsystem(sk) = a {
                            Some(sk)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

    println!("\n{}\n", "Skimsystems:".bold());

    for sk in &filtered {
        let status_dot = match sk.observation_statuses.overall() {
            crate::ast::Status::Green => "●".green(),
            crate::ast::Status::Yellow => "●".yellow(),
            crate::ast::Status::Red => "●".red(),
        };

        println!(
            "  {} {} (owner: {})",
            status_dot,
            sk.name.bold(),
            sk.owner
        );

        // Show targets from declaration
        if let Some(decl) = skimsystem_decls.iter().find(|d| d.name == sk.name) {
            let targets_str = match &decl.targets {
                crate::ast::SkimTargets::All => "all".to_string(),
                crate::ast::SkimTargets::Named(names) => names.join(", "),
            };
            println!("    Targets: {targets_str}");

            if !decl.principles.is_empty() {
                println!("    Principles:");
                for p in &decl.principles {
                    println!("      - {p}");
                }
            }
        }

        if sk.observation_count > 0 {
            println!(
                "    Observations: {} ({} green, {} yellow, {} red)",
                sk.observation_count,
                sk.observation_statuses.green,
                sk.observation_statuses.yellow,
                sk.observation_statuses.red
            );
        } else {
            println!("    Observations: none");
        }

        for (subsys, counts) in &sk.subsystem_coverage {
            if counts.total() > 0 {
                let sub_dot = match counts.overall() {
                    crate::ast::Status::Green => "●".green(),
                    crate::ast::Status::Yellow => "●".yellow(),
                    crate::ast::Status::Red => "●".red(),
                };
                println!(
                    "    {sub_dot} {subsys} ({} observations)",
                    counts.total()
                );
            }
        }

        // Verbose: show individual observations from .bog files
        if verbose {
            println!("    Details:");
            let pattern = root.join("**/*.bog");
            if let Ok(paths) = glob::glob(&pattern.to_string_lossy()) {
                for bog_path in paths.flatten() {
                    let rel = bog_path.strip_prefix(root).unwrap_or(&bog_path);
                    if rel.components().any(|c| {
                        matches!(c.as_os_str().to_str(), Some("target" | ".git"))
                    }) {
                        continue;
                    }
                    if bog_path.file_name().map(|n| n == "repo.bog").unwrap_or(false) {
                        continue;
                    }

                    if let Ok(content) = std::fs::read_to_string(&bog_path) {
                        if let Ok(bog) = crate::parser::parse_bog(&content) {
                            for ann in &bog.annotations {
                                if let crate::ast::Annotation::Skim(obs) = ann {
                                    if obs.skimsystem == sk.name {
                                        let file_rel = rel.to_string_lossy();
                                        let dot = match obs.status {
                                            crate::ast::Status::Green => "●".green(),
                                            crate::ast::Status::Yellow => "●".yellow(),
                                            crate::ast::Status::Red => "●".red(),
                                        };
                                        let target_str = match &obs.target {
                                            Some(crate::ast::SkimTarget::Fn(name)) => {
                                                format!(" -> fn({name})")
                                            }
                                            _ => String::new(),
                                        };
                                        println!(
                                            "      {dot} {file_rel}{target_str}"
                                        );
                                        if let Some(notes) = &obs.notes {
                                            println!("        \"{notes}\"");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        println!();
    }

    Ok(())
}

fn cmd_stub(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Generating stubs for unannotated functions...".bold());

    let missing = stub::find_missing_annotations(root);
    if missing.is_empty() {
        println!("  {}", "All functions are annotated.".green().bold());
        return Ok(());
    }

    // Preview what will be generated
    for (source_path, _bog_path, symbols) in &missing {
        let rel = source_path
            .strip_prefix(root)
            .unwrap_or(source_path)
            .display();
        for sym in symbols {
            let deps = if sym.calls.is_empty() {
                String::new()
            } else {
                format!(" (deps: {})", sym.calls.join(", "))
            };
            println!("  {} {}{deps}", rel, sym.name.bold());
        }
    }

    let report = stub::apply_stubs(root);

    println!(
        "\n  {} stub(s) generated across {} file(s) ({} modified, {} created).",
        report.stubs_generated.to_string().bold(),
        (report.files_modified + report.files_created)
            .to_string()
            .bold(),
        report.files_modified,
        report.files_created,
    );
    println!(
        "  Run {} to see them. Fill in and remove {}.",
        "bogbot stub --list".bold(),
        "stub = true".bold()
    );

    Ok(())
}

fn cmd_stub_list(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let stubs = stub::list_stubs(root);
    if stubs.is_empty() {
        println!("{}", "No stub annotations found.".green().bold());
        return Ok(());
    }

    println!("{} stub annotation(s):\n", stubs.len().to_string().bold());

    let mut current_file = "";
    for (file, func) in &stubs {
        if file != current_file {
            println!("  {}:", file.bold());
            current_file = file;
        }
        println!("    {} {func}", "stub:".yellow());
    }

    println!(
        "\nRemove {} from each annotation when complete.",
        "stub = true".bold()
    );

    Ok(())
}
