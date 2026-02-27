use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use colored::Colorize;

use crate::context;
use crate::health;
use crate::orchestrate;
use crate::stub;
use crate::validator;

#[derive(Parser)]
#[command(name = "bog", version, about = "Agent-first codebase annotation system")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize bog in the current directory
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

    /// Show skimsystem health, run integrations, or check principles
    Skim {
        /// Path to project root (defaults to current directory)
        path: Option<PathBuf>,

        /// Target a specific skimsystem (triggers run mode)
        #[arg(long)]
        name: Option<String>,

        /// Run a specific action: integration name (e.g. "clippy") or "check" for principles
        #[arg(long)]
        action: Option<String>,

        /// Show individual observations
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show annotation context scoped to an agent or subsystem
    Context {
        /// Path to project root (defaults to current directory)
        path: Option<PathBuf>,

        /// Scope to a specific agent's subsystem(s)
        #[arg(long)]
        agent: Option<String>,

        /// Scope to a specific subsystem
        #[arg(long)]
        subsystem: Option<String>,

        /// Show only pickled annotations
        #[arg(long)]
        pickled: bool,

        /// Show only change_requests
        #[arg(long)]
        requests: bool,

        /// Show only health dimensions
        #[arg(long)]
        health: bool,

        /// Show only function contracts
        #[arg(long)]
        contracts: bool,

        /// Show only skim observations
        #[arg(long)]
        skims: bool,

        /// Filter pickled by kind (decision, reversal, context, observation, rationale)
        #[arg(long)]
        kind: Option<String>,

        /// Filter pickled by tag (architecture, performance, reliability, security, testing, domain, debt, tooling)
        #[arg(long)]
        tag: Option<String>,

        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Generate stub annotations for unannotated functions
    Stub {
        /// Path to process (defaults to current directory)
        path: Option<PathBuf>,

        /// List all current stub annotations instead of generating new ones
        #[arg(long)]
        list: bool,
    },

    /// Multi-agent orchestration: delegate work to subsystem agents
    Orchestrate {
        #[command(subcommand)]
        command: OrchestrateCommand,

        /// Path to project root (defaults to current directory)
        #[arg(short, long, global = true)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum OrchestrateCommand {
    /// Run a free-form request through the dock → delegate → merge loop.
    Run {
        /// The request to process.
        request: String,

        /// Maximum replan attempts after violations.
        #[arg(long, default_value = "2")]
        max_replans: usize,

        /// Merge strategy: "incremental" or "all-or-nothing".
        #[arg(long, default_value = "all-or-nothing")]
        merge_strategy: String,

        /// Just produce the dock plan without executing (dry run).
        #[arg(long)]
        plan_only: bool,
    },

    /// Run a skimsystem lifecycle: integrate → delegate → resolve → close.
    Skim {
        /// Skimsystem name (e.g., "code-quality").
        name: String,

        /// Specific integration action to run (e.g., "clippy").
        #[arg(long)]
        action: Option<String>,
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
        Command::Skim {
            path,
            name,
            action,
            verbose,
        } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            if let Some(ref name) = name {
                cmd_skim_run(&root, name, action.as_deref())
            } else {
                cmd_skim(&root, None, verbose)
            }
        }
        Command::Context {
            path,
            agent,
            subsystem,
            pickled,
            requests,
            health,
            contracts,
            skims,
            kind,
            tag,
            format,
        } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            let scope = match (agent, subsystem) {
                (Some(a), _) => context::ContextScope::Agent(a),
                (_, Some(s)) => context::ContextScope::Subsystem(s),
                _ => context::ContextScope::All,
            };
            let any_flag = pickled || requests || health || contracts || skims;
            let filter = if any_flag {
                context::SectionFilter { pickled, requests, health, contracts, skims }
            } else {
                context::SectionFilter::all()
            };
            cmd_context(&root, scope, filter, kind.as_deref(), tag.as_deref(), &format)
        }
        Command::Stub { path, list } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            if list {
                cmd_stub_list(&root)
            } else {
                cmd_stub(&root)
            }
        }
        Command::Orchestrate { command, path } => {
            let root = path
                .unwrap_or_else(|| PathBuf::from("."))
                .canonicalize()?;
            let ctx = orchestrate::context::RepoContext::load(&root)?;
            eprintln!(
                "{} Loaded {} subsystems, {} skimsystems, {} agents",
                "bog orchestrate:".bold(),
                ctx.subsystems.len(),
                ctx.skimsystems.len(),
                ctx.derived_agents.roles.len(),
            );
            let provider = orchestrate::provider::ClaudeCliProvider;
            match command {
                OrchestrateCommand::Skim { name, action } => {
                    cmd_orchestrate_skim(&ctx, &name, action.as_deref(), &provider)
                }
                OrchestrateCommand::Run {
                    request,
                    max_replans,
                    merge_strategy,
                    plan_only,
                } => {
                    if plan_only {
                        cmd_orchestrate_plan_only(&ctx, &request, &provider)
                    } else {
                        cmd_orchestrate_run(&ctx, &request, &provider, max_replans, &merge_strategy)
                    }
                }
            }
        }
    }
}

fn cmd_init() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(".");

    // Create bog.toml
    let config_path = root.join("bog.toml");
    if config_path.exists() {
        println!("{} bog.toml already exists, skipping", "note:".yellow());
    } else {
        std::fs::write(
            &config_path,
            r#"[bog]
version = "0.1.0"

[tree_sitter]
language = "rust"

[health]
dimensions = ["test_coverage", "staleness", "complexity", "contract_compliance"]
"#,
        )?;
        println!("{} created bog.toml", "ok:".green());
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
        "\n{} Edit repo.bog to declare subsystems and their owning agents.",
        "next:".bold()
    );
    println!(
        "      Edit bog.toml to configure tree-sitter and health dimensions.\n      Run {} to verify.",
        "bog validate".bold()
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
        "bog skim".bold()
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
                    | validator::ValidationError::UndeclaredSkimsystem { .. }
                    | validator::ValidationError::SkimsystemTargetNotFound { .. }
                    | validator::ValidationError::SkimTargetFunctionMissing { .. }
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

fn cmd_skim_run(
    root: &Path,
    name: &str,
    action_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::integration;

    // Parse repo.bog to find the target skimsystem
    let repo_bog_path = root.join("repo.bog");
    let content = std::fs::read_to_string(&repo_bog_path)?;
    let bog = crate::parser::parse_bog(&content)?;

    let skimsystem = bog
        .annotations
        .into_iter()
        .find_map(|a| {
            if let crate::ast::Annotation::Skimsystem(sk) = a {
                if sk.name == name {
                    return Some(sk);
                }
            }
            None
        })
        .ok_or_else(|| format!("skimsystem '{name}' not found in repo.bog"))?;

    // Determine which actions to run
    let run_check = action_filter.is_none() || action_filter == Some("check");
    let run_integrations: Vec<&crate::ast::IntegrationSpec> = if let Some(action) = action_filter {
        if action == "check" {
            Vec::new()
        } else {
            match skimsystem.integrations.iter().find(|i| i.name == action) {
                Some(spec) => vec![spec],
                None => {
                    println!(
                        "  {} integration '{}' not found in skimsystem '{}'",
                        "error:".red(),
                        action,
                        name
                    );
                    println!(
                        "  Available: {}",
                        skimsystem
                            .integrations
                            .iter()
                            .map(|i| i.name.as_str())
                            .chain(std::iter::once("check"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    std::process::exit(1);
                }
            }
        }
    } else {
        skimsystem.integrations.iter().collect()
    };

    println!(
        "{} {} {}",
        "Running skimsystem".bold(),
        name.bold(),
        "...".bold()
    );

    // Run check (principles-based) pass
    if run_check {
        println!("\n  {} check (principles)", ">>".bold());
        for p in &skimsystem.principles {
            println!("    - {p}");
        }
        println!("    {}", "(principles listed for agent reference)".dimmed());
    }

    // Run each integration
    for spec in &run_integrations {
        println!("\n  {} {} integration", ">>".bold(), spec.name.bold());
        println!("  $ {}", spec.command.dimmed());

        let mut report =
            integration::run_integration(&skimsystem.name, &spec.name, spec, root)?;

        if report.build_error.is_some() {
            integration::print_report(&report);
            continue;
        }

        // Write results to .bog files
        integration::write_integration_results(
            &skimsystem.name,
            &spec.name,
            &skimsystem.owner,
            &mut report,
            root,
        )?;

        integration::print_report(&report);
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
        "bog stub --list".bold(),
        "stub = true".bold()
    );

    Ok(())
}

fn cmd_context(
    root: &Path,
    scope: context::ContextScope,
    filter: context::SectionFilter,
    kind_filter: Option<&str>,
    tag_filter: Option<&str>,
    format: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = context::load_context(root, scope, filter, kind_filter, tag_filter)?;

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            print!("{}", context::format_context_text(&output));
        }
    }

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

fn cmd_orchestrate_skim(
    ctx: &orchestrate::context::RepoContext,
    name: &str,
    action: Option<&str>,
    provider: &dyn orchestrate::provider::Provider,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = orchestrate::skim::run_skim_lifecycle(ctx, name, action, provider)?;

    println!();
    if result.work_packets.is_empty() {
        println!(
            "{} No pending change_requests from '{}'.",
            "OK:".green().bold(),
            result.skimsystem
        );
        return Ok(());
    }

    if result.merged {
        println!(
            "{} All subsystem agents completed. Changes merged.",
            "OK:".green().bold(),
        );
    } else {
        println!(
            "{} Skim lifecycle failed for '{}'.",
            "FAIL:".red().bold(),
            result.skimsystem
        );
        for (agent, violations) in &result.violations {
            println!("  Agent '{agent}':");
            for v in violations {
                println!("    - {}: {}", v.file_path, v.reason);
            }
        }
    }

    println!();
    println!("{}", "Agent results:".bold());
    for r in &result.agent_results {
        let status_str = match &r.status {
            orchestrate::plan::AgentResultStatus::Success => "OK".green().to_string(),
            orchestrate::plan::AgentResultStatus::Failed(msg) => format!("{} {msg}", "FAIL".red()),
            orchestrate::plan::AgentResultStatus::PermissionViolation(_) => {
                "DENIED".red().to_string()
            }
        };
        println!(
            "  [{status_str}] {} (task {}): {} files modified",
            r.agent,
            r.task_index,
            r.files_modified.len()
        );
    }

    if !result.merged {
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_orchestrate_plan_only(
    ctx: &orchestrate::context::RepoContext,
    request: &str,
    provider: &dyn orchestrate::provider::Provider,
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = orchestrate::dock::run_dock(ctx, request, provider, None)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn cmd_orchestrate_run(
    ctx: &orchestrate::context::RepoContext,
    request: &str,
    provider: &dyn orchestrate::provider::Provider,
    max_replans: usize,
    merge_strategy: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = orchestrate::orchestrator::OrchestrateConfig {
        max_replan_attempts: max_replans,
        merge_strategy: match merge_strategy {
            "incremental" => orchestrate::orchestrator::MergeStrategy::Incremental,
            _ => orchestrate::orchestrator::MergeStrategy::AllOrNothing,
        },
    };

    let result = orchestrate::orchestrator::orchestrate(ctx, request, provider, &config)?;

    println!();
    if result.merged {
        println!(
            "{} All agent changes merged successfully.",
            "OK:".green().bold(),
        );
    } else {
        println!("{} Orchestration failed.", "FAIL:".red().bold());
        for (agent, violations) in &result.violations {
            println!("  Agent '{agent}':");
            for v in violations {
                println!("    - {}: {}", v.file_path, v.reason);
            }
        }
    }

    println!();
    println!("{}", "Agent results:".bold());
    for r in &result.agent_results {
        let status_str = match &r.status {
            orchestrate::plan::AgentResultStatus::Success => "OK".green().to_string(),
            orchestrate::plan::AgentResultStatus::Failed(msg) => format!("{} {msg}", "FAIL".red()),
            orchestrate::plan::AgentResultStatus::PermissionViolation(_) => {
                "DENIED".red().to_string()
            }
        };
        println!(
            "  [{status_str}] {} (task {}): {} files modified",
            r.agent,
            r.task_index,
            r.files_modified.len()
        );
    }

    if !result.merged {
        std::process::exit(1);
    }

    Ok(())
}
