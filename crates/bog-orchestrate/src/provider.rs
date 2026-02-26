use std::path::Path;
use std::process::Command;

use crate::error::ProviderError;

/// Output from a provider invocation.
#[derive(Debug, Clone)]
pub struct ProviderOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Options controlling provider invocation.
#[derive(Debug, Clone)]
pub struct ProviderOptions {
    pub timeout_seconds: u64,
    pub model: Option<String>,
    /// Whether this invocation should be read-only (dock agent).
    pub read_only: bool,
    /// Allowed tools for Claude CLI.
    pub allowed_tools: Option<Vec<String>>,
    /// Maximum dollar budget for this invocation.
    pub max_budget_usd: Option<f64>,
}

impl Default for ProviderOptions {
    fn default() -> Self {
        Self {
            timeout_seconds: 300,
            model: None,
            read_only: false,
            allowed_tools: None,
            max_budget_usd: None,
        }
    }
}

/// Trait for invoking an LLM provider.
pub trait Provider: Send + Sync {
    fn invoke(
        &self,
        prompt: &str,
        system_prompt: &str,
        working_dir: &Path,
        options: &ProviderOptions,
    ) -> Result<ProviderOutput, ProviderError>;
}

/// Claude CLI implementation of the Provider trait.
pub struct ClaudeCliProvider;

impl Provider for ClaudeCliProvider {
    fn invoke(
        &self,
        prompt: &str,
        system_prompt: &str,
        working_dir: &Path,
        options: &ProviderOptions,
    ) -> Result<ProviderOutput, ProviderError> {
        let mut cmd = Command::new("claude");
        cmd.arg("-p").arg(prompt);
        cmd.arg("--system-prompt").arg(system_prompt);
        cmd.arg("--output-format").arg("json");

        if let Some(ref model) = options.model {
            cmd.arg("--model").arg(model);
        }

        if let Some(ref tools) = options.allowed_tools {
            cmd.arg("--allowedTools").arg(tools.join(","));
        }

        if let Some(budget) = options.max_budget_usd {
            cmd.arg("--max-turns")
                .arg(budget_to_turns(budget).to_string());
        }

        cmd.current_dir(working_dir);

        // Clear the CLAUDECODE env var to allow nested Claude CLI sessions.
        // The orchestrator spawns Claude as subprocesses — not true nesting.
        cmd.env_remove("CLAUDECODE");

        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProviderError::CliNotFound
            } else {
                ProviderError::Io(e)
            }
        })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(ProviderOutput {
            stdout,
            stderr,
            exit_code,
        })
    }
}

/// Rough heuristic: convert a dollar budget to max turns.
fn budget_to_turns(budget_usd: f64) -> u32 {
    // ~$0.05 per turn as a rough estimate
    (budget_usd / 0.05).ceil().max(1.0) as u32
}

/// Mock provider for testing.
#[cfg(test)]
pub struct MockProvider {
    pub response: String,
}

#[cfg(test)]
impl Provider for MockProvider {
    fn invoke(
        &self,
        _prompt: &str,
        _system_prompt: &str,
        _working_dir: &Path,
        _options: &ProviderOptions,
    ) -> Result<ProviderOutput, ProviderError> {
        Ok(ProviderOutput {
            stdout: self.response.clone(),
            stderr: String::new(),
            exit_code: 0,
        })
    }
}
