use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// A launchable agent CLI (spawned in a PTY).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCmd {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Connection details for LM Studio's local OpenAI-compatible server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LmConfig {
    pub base_url: String,
    pub model: String,
}

/// Default run parameters that back the scrubbers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    pub temperature: f64,
    pub max_tokens: f64,
    pub top_p: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub claude: AgentCmd,
    pub codex: AgentCmd,
    pub lmstudio: LmConfig,
    pub params: Params,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude: AgentCmd {
                command: "claude".to_string(),
                args: vec![],
            },
            codex: AgentCmd {
                command: "codex".to_string(),
                args: vec![],
            },
            lmstudio: LmConfig {
                base_url: "http://localhost:1234/v1".to_string(),
                model: "local-model".to_string(),
            },
            params: Params {
                temperature: 0.7,
                max_tokens: 2048.0,
                top_p: 1.0,
            },
        }
    }
}

impl Config {
    /// Load config from disk, creating a default one if none exists.
    pub fn load_or_default() -> Self {
        match Self::path().and_then(|p| Self::from_path(&p)) {
            Ok(cfg) => cfg,
            Err(_) => {
                let cfg = Config::default();
                let _ = cfg.save();
                cfg
            }
        }
    }

    fn from_path(path: &PathBuf) -> Result<Self> {
        let text = std::fs::read_to_string(path).context("read config")?;
        let cfg = toml::from_str(&text).context("parse config")?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let text = toml::to_string_pretty(self).context("serialize config")?;
        std::fs::write(&path, text).context("write config")?;
        Ok(())
    }

    fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("dev", "mint", "mint-cli")
            .context("could not resolve config directory")?;
        Ok(dirs.config_dir().join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn default_config_has_expected_commands() {
        let cfg = Config::default();
        assert_eq!(cfg.claude.command, "claude");
        assert_eq!(cfg.codex.command, "codex");
        assert_eq!(cfg.lmstudio.base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.params.temperature, cfg.params.temperature);
        assert_eq!(parsed.lmstudio.model, cfg.lmstudio.model);
    }
}
