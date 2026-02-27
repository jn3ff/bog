use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
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
    /// Label for progress output (e.g., agent name). None = silent.
    pub agent_label: Option<String>,
}

impl Default for ProviderOptions {
    fn default() -> Self {
        Self {
            timeout_seconds: 300,
            model: None,
            read_only: false,
            allowed_tools: None,
            max_budget_usd: None,
            agent_label: None,
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
        cmd.arg("--output-format").arg("stream-json");
        cmd.arg("--verbose");

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

        let label = options.agent_label.clone().unwrap_or_default();

        // Collect stderr in background (not much comes here with stream-json, but avoid deadlock)
        let stderr_pipe = child.stderr.take();
        let stderr_thread = std::thread::spawn(move || {
            let mut collected = String::new();
            if let Some(pipe) = stderr_pipe {
                let reader = BufReader::new(pipe);
                for line in reader.lines().flatten() {
                    collected.push_str(&line);
                    collected.push('\n');
                }
            }
            collected
        });

        // Parse stream-json events from stdout: emit progress, collect result
        let stdout_pipe = child.stdout.take();
        let progress_label = label.clone();
        let stdout_thread =
            std::thread::spawn(move || parse_event_stream(stdout_pipe, &progress_label));

        // Wait with timeout
        let timeout = Duration::from_secs(options.timeout_seconds);
        let start = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let progress = stdout_thread.join().unwrap_or_default();
                    let stderr = stderr_thread.join().unwrap_or_default();
                    let exit_code = status.code().unwrap_or(-1);

                    // Print summary line
                    let failed = progress.is_error || exit_code != 0;
                    if !label.is_empty() {
                        let elapsed = start.elapsed().as_secs();
                        let turns = progress.num_turns.unwrap_or(progress.turn_count);
                        let cost = progress
                            .cost_usd
                            .map(|c| format!(", ${c:.2}"))
                            .unwrap_or_default();
                        if failed {
                            eprintln!("  [{label}] ✗ failed (exit {exit_code}) — {elapsed}s, {turns} turns{cost}");
                            if !stderr.is_empty() {
                                for line in stderr.lines().take(10) {
                                    eprintln!("  [{label}]   {line}");
                                }
                            }
                        } else {
                            eprintln!("  [{label}] ✓ done — {elapsed}s, {turns} turns{cost}");
                        }
                    }

                    // Return the result text (not raw NDJSON) so callers parse it directly
                    let result_text = progress
                        .result_text
                        .or(progress.last_assistant_text)
                        .unwrap_or_default();

                    return Ok(ProviderOutput {
                        stdout: result_text,
                        stderr,
                        exit_code,
                    });
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        if !label.is_empty() {
                            eprintln!(
                                "  [{label}] ✗ timed out after {}s",
                                options.timeout_seconds
                            );
                        }
                        let _ = child.kill();
                        let _ = child.wait();
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

// ---------------------------------------------------------------------------
// Stream-json event parsing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StreamProgress {
    result_text: Option<String>,
    cost_usd: Option<f64>,
    num_turns: Option<u32>,
    is_error: bool,
    /// Fallback if the result event is missing (known Claude CLI bug).
    last_assistant_text: Option<String>,
    turn_count: u32,
}

/// Read NDJSON events from the Claude CLI stream-json output.
/// Emits per-turn progress to stderr and collects the final result.
fn parse_event_stream(pipe: Option<ChildStdout>, label: &str) -> StreamProgress {
    let mut progress = StreamProgress::default();
    let Some(pipe) = pipe else {
        return progress;
    };

    let reader = BufReader::new(pipe);
    for line in reader.lines().flatten() {
        if line.trim().is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        match event["type"].as_str() {
            Some("assistant") => {
                progress.turn_count += 1;
                let mut tools: Vec<String> = Vec::new();
                let mut text = String::new();

                if let Some(content) = event["message"]["content"].as_array() {
                    for block in content {
                        match block["type"].as_str() {
                            Some("tool_use") => {
                                let name = block["name"].as_str().unwrap_or("?");
                                let summary = summarize_tool_input(name, &block["input"]);
                                if summary.is_empty() {
                                    tools.push(name.to_string());
                                } else {
                                    tools.push(format!("{name} {summary}"));
                                }
                            }
                            Some("text") => {
                                if let Some(t) = block["text"].as_str() {
                                    text = t.to_string();
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if !text.is_empty() {
                    progress.last_assistant_text = Some(text);
                }

                // Emit progress for turns that use tools
                if !tools.is_empty() && !label.is_empty() {
                    let turn = progress.turn_count;
                    eprintln!("  [{label}] turn {turn} ▸ {}", tools.join(", "));
                }
            }
            Some("result") => {
                progress.result_text = event["result"].as_str().map(String::from);
                progress.cost_usd = event["cost_usd"].as_f64();
                progress.num_turns = event["num_turns"].as_u64().map(|n| n as u32);
                progress.is_error = event["is_error"].as_bool().unwrap_or(false);
            }
            _ => {} // system, user events — skip
        }
    }

    progress
}

/// Extract a short summary of a tool's input for progress display.
fn summarize_tool_input(tool: &str, input: &serde_json::Value) -> String {
    match tool {
        "Read" | "Edit" | "Write" => {
            let path = input["file_path"].as_str().unwrap_or("");
            strip_worktree_prefix(path).to_string()
        }
        "Bash" => {
            let cmd = input["command"].as_str().unwrap_or("").trim();
            if cmd.chars().count() > 60 {
                let short: String = cmd.chars().take(60).collect();
                format!("{short}…")
            } else {
                cmd.to_string()
            }
        }
        "Grep" => {
            let pattern = input["pattern"].as_str().unwrap_or("");
            let path = strip_worktree_prefix(input["path"].as_str().unwrap_or("."));
            format!("/{pattern}/ {path}")
        }
        "Glob" => input["pattern"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Strip `.bog-worktrees/<uuid>/<agent>/` prefix from absolute paths.
fn strip_worktree_prefix(path: &str) -> &str {
    if let Some(idx) = path.find(".bog-worktrees/") {
        let after = &path[idx..];
        // Skip past .bog-worktrees/<uuid>/<agent>/
        let mut slashes = 0;
        for (i, ch) in after.char_indices() {
            if ch == '/' {
                slashes += 1;
                if slashes == 3 {
                    return &after[i + 1..];
                }
            }
        }
    }
    path
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
