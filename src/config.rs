use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BogbotConfig {
    pub bogbot: BogbotMeta,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    #[serde(default)]
    pub tree_sitter: TreeSitterConfig,
    #[serde(default)]
    pub health: HealthConfig,
}

#[derive(Debug, Deserialize)]
pub struct BogbotMeta {
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    pub description: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct TreeSitterConfig {
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "rust".to_string()
}

#[derive(Debug, Deserialize, Default)]
pub struct HealthConfig {
    #[serde(default)]
    pub dimensions: Vec<String>,
}

pub fn load_config(path: &Path) -> Result<BogbotConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let config: BogbotConfig = toml::from_str(&content)?;
    Ok(config)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
}
