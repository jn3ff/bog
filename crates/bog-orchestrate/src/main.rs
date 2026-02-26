use std::path::PathBuf;

use clap::{Parser, Subcommand};
use colored::Colorize;

use bog_orchestrate::context::RepoContext;
use bog_orchestrate::dock;
use bog_orchestrate::orchestrator::{self, MergeStrategy, OrchestrateConfig};
use bog_orchestrate::plan::AgentResultStatus;
use bog_orchestrate::provider::ClaudeCliProvider;
use bog_orchestrate::skim;

#[derive(Parser)]
#[command(
    name = "bog-orchestrate",
    version,
    about = "Multi-agent orchestration for bog"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to project root (defaults to current directory).
    #[arg(short, long, global = true)]
    path: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a skimsystem lifecycle: integrate → delegate → resolve → close.
    /// No manual prompt needed — the skimsystem drives everything.
    Skim {
        /// Skimsystem name (e.g., "code-quality").
        name: String,

        /// Specific integration action to run (e.g., "clippy").
        #[arg(long)]
        action: Option<String>,
    },

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let root = cli
        .path
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()?;

    let ctx = RepoContext::load(&root)?;
    eprintln!(
        "{} Loaded {} subsystems, {} skimsystems, {} agents",
        "bog-orchestrate:".bold(),
        ctx.subsystems.len(),
        ctx.skimsystems.len(),
        ctx.config.agents.len(),
    );

    let provider = ClaudeCliProvider;

    match cli.command {
        Commands::Skim { name, action } => {
            cmd_skim(&ctx, &name, action.as_deref(), &provider)
        }
        Commands::Run {
            request,
            max_replans,
            merge_strategy,
            plan_only,
        } => {
            if plan_only {
                cmd_plan_only(&ctx, &request, &provider)
            } else {
                cmd_run(&ctx, &request, &provider, max_replans, &merge_strategy)
            }
        }
    }
}

fn cmd_skim(
    ctx: &RepoContext,
    name: &str,
    action: Option<&str>,
    provider: &dyn bog_orchestrate::provider::Provider,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = skim::run_skim_lifecycle(ctx, name, action, provider)?;

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
            AgentResultStatus::Success => "OK".green().to_string(),
            AgentResultStatus::Failed(msg) => format!("{} {msg}", "FAIL".red()),
            AgentResultStatus::PermissionViolation(_) => "DENIED".red().to_string(),
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

fn cmd_plan_only(
    ctx: &RepoContext,
    request: &str,
    provider: &dyn bog_orchestrate::provider::Provider,
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = dock::run_dock(ctx, request, provider, None)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

fn cmd_run(
    ctx: &RepoContext,
    request: &str,
    provider: &dyn bog_orchestrate::provider::Provider,
    max_replans: usize,
    merge_strategy: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = OrchestrateConfig {
        max_replan_attempts: max_replans,
        merge_strategy: match merge_strategy {
            "incremental" => MergeStrategy::Incremental,
            _ => MergeStrategy::AllOrNothing,
        },
    };

    let result = orchestrator::orchestrate(ctx, request, provider, &config)?;

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
            AgentResultStatus::Success => "OK".green().to_string(),
            AgentResultStatus::Failed(msg) => format!("{} {msg}", "FAIL".red()),
            AgentResultStatus::PermissionViolation(_) => "DENIED".red().to_string(),
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
