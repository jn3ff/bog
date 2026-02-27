use std::io::{BufRead, BufReader, Read as _};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::error::ProviderError;

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

        // Apply max-turns: budget heuristic or a default cap to prevent unbounded runs
        let max_turns = options
            .max_budget_usd
            .map(budget_to_turns)
            .unwrap_or(50);
        cmd.arg("--max-turns").arg(max_turns.to_string());

        cmd.current_dir(working_dir);

        // Clear the CLAUDECODE env var to allow nested Claude CLI sessions.
        // The orchestrator spawns Claude as subprocesses — not true nesting.
        cmd.env_remove("CLAUDECODE");

        // Pipe stdout (capture JSON response), pipe stderr (stream to terminal), null stdin
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProviderError::CliNotFound
            } else {
                ProviderError::Io(e)
            }
        })?;

        // Stream stderr to terminal in a background thread
        let stderr_pipe = child.stderr.take();
        let stderr_thread = std::thread::spawn(move || {
            let mut collected = String::new();
            if let Some(pipe) = stderr_pipe {
                let reader = BufReader::new(pipe);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            eprintln!("  [agent] {line}");
                            collected.push_str(&line);
                            collected.push('\n');
                        }
                        Err(_) => break,
                    }
                }
            }
            collected
        });

        // Capture stdout in a background thread (avoids pipe buffer deadlock)
        let stdout_pipe = child.stdout.take();
        let stdout_thread = std::thread::spawn(move || {
            let mut collected = String::new();
            if let Some(mut pipe) = stdout_pipe {
                let _ = pipe.read_to_string(&mut collected);
            }
            collected
        });

        // Wait with timeout
        let timeout = Duration::from_secs(options.timeout_seconds);
        let start = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = stdout_thread.join().unwrap_or_default();
                    let stderr = stderr_thread.join().unwrap_or_default();
                    let exit_code = status.code().unwrap_or(-1);

                    return Ok(ProviderOutput {
                        stdout,
                        stderr,
                        exit_code,
                    });
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        let _ = child.wait(); // reap the process
                        return Err(ProviderError::Timeout {
                            seconds: options.timeout_seconds,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(ProviderError::Io(e)),
            }
        }
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
