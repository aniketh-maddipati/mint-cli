use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// A launchable CLI (spawned in a PTY).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCmd {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Optional command pane declared in config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPane {
    pub label: String,
    #[serde(flatten)]
    pub cmd: AgentCmd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub claude: AgentCmd,
    pub codex: AgentCmd,
    #[serde(default)]
    pub commands: Vec<CommandPane>,
}

/// Legacy config shapes for migration.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    claude: AgentCmd,
    codex: AgentCmd,
    #[serde(default)]
    http: Option<LegacyHttpSection>,
    params: Option<LegacyParams>,
}

#[derive(Debug, Deserialize)]
struct LegacyHttpSection {
    tinker: Option<LegacyHttpConfig>,
    lmstudio: Option<LegacyHttpConfig>,
}

#[derive(Debug, Deserialize)]
struct LegacyHttpConfig {
    base_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct LegacyParams {
    temperature: f64,
    max_tokens: f64,
    top_p: f64,
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
            commands: vec![],
        }
    }
}

impl Config {
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
        if let Ok(cfg) = toml::from_str::<Config>(&text) {
            return Ok(cfg);
        }
        // Migrate legacy chat-era config (drops http/params).
        if let Ok(legacy) = toml::from_str::<LegacyConfig>(&text) {
            let cfg = Config {
                claude: legacy.claude,
                codex: legacy.codex,
                commands: vec![],
            };
            let _ = cfg.save();
            return Ok(cfg);
        }
        anyhow::bail!("parse config")
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
        assert!(cfg.commands.is_empty());
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.claude.command, cfg.claude.command);
    }
}
