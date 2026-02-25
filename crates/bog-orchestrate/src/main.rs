use std::path::PathBuf;

use clap::Parser;
use colored::Colorize;

use bog_orchestrate::context::RepoContext;
use bog_orchestrate::dock;
use bog_orchestrate::orchestrator::{self, MergeStrategy, OrchestrateConfig};
use bog_orchestrate::plan::AgentResultStatus;
use bog_orchestrate::provider::ClaudeCliProvider;

#[derive(Parser)]
#[command(
    name = "bog-orchestrate",
    version,
    about = "Multi-agent orchestration for bog"
)]
struct Cli {
    /// The request to process.
    request: String,

    /// Path to project root (defaults to current directory).
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// Maximum replan attempts after violations.
    #[arg(long, default_value = "2")]
    max_replans: usize,

    /// Merge strategy: "incremental" or "all-or-nothing".
    #[arg(long, default_value = "all-or-nothing")]
    merge_strategy: String,

    /// Just produce the dock plan without executing (dry run).
    #[arg(long)]
    plan_only: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let root = cli
        .path
        .unwrap_or_else(|| PathBuf::from("."))
        .canonicalize()?;

    // Load repo context
    let ctx = RepoContext::load(&root)?;
    eprintln!(
        "{} Loaded {} subsystems, {} skimsystems, {} agents",
        "bog-orchestrate:".bold(),
        ctx.subsystems.len(),
        ctx.skimsystems.len(),
        ctx.config.agents.len(),
    );

    let provider = ClaudeCliProvider;

    if cli.plan_only {
        let plan = dock::run_dock(&ctx, &cli.request, &provider, None)?;
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }

    let config = OrchestrateConfig {
        max_replan_attempts: cli.max_replans,
        merge_strategy: match cli.merge_strategy.as_str() {
            "incremental" => MergeStrategy::Incremental,
            _ => MergeStrategy::AllOrNothing,
        },
    };

    let result = orchestrator::orchestrate(&ctx, &cli.request, &provider, &config)?;

    // Print results
    println!();
    if result.merged {
        println!(
            "{} {}",
            "OK:".green().bold(),
            "All agent changes merged successfully."
        );
    } else {
        println!("{} {}", "FAIL:".red().bold(), "Orchestration failed.");
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
