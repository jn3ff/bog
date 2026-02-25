use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error("Failed to load repo context: {0}")]
    ContextLoad(String),

    #[error("Dock agent failed: {0}")]
    DockFailed(String),

    #[error("Dock plan is invalid: {0}")]
    InvalidPlan(String),

    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),

    #[error("Worktree error: {0}")]
    Worktree(#[from] WorktreeError),

    #[error("Permission violations for agent '{agent}'")]
    PermissionViolation {
        agent: String,
        violations: Vec<crate::permissions::Violation>,
    },

    #[error("Agent '{agent}' execution failed: {message}")]
    AgentFailed { agent: String, message: String },

    #[error("All replan attempts exhausted after {attempts} tries")]
    ReplanExhausted { attempts: usize },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Claude CLI not found on PATH")]
    CliNotFound,

    #[error("Claude CLI exited with code {code}: {stderr}")]
    CliExitError { code: i32, stderr: String },

    #[error("Failed to parse provider output: {0}")]
    OutputParse(String),

    #[error("Provider timeout after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("Failed to create worktree at {path}: {message}")]
    CreateFailed { path: PathBuf, message: String },

    #[error("Failed to remove worktree at {path}: {message}")]
    RemoveFailed { path: PathBuf, message: String },

    #[error("Git command failed: {0}")]
    GitFailed(String),
}
