use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::health;
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
# my-agent = { description = "Owns some subsystem" }

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
