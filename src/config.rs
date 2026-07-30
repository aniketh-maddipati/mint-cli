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

/// OpenAI-compatible HTTP endpoint (LM Studio, Tinker, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfigs {
    pub tinker: HttpConfig,
    pub lmstudio: HttpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Params {
    pub temperature: f64,
    pub max_tokens: f64,
    pub top_p: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub claude: AgentCmd,
    pub codex: AgentCmd,
    #[serde(default, alias = "lmstudio")]
    pub http: HttpConfigs,
    pub params: Params,
}

/// Legacy config shape for migration.
#[derive(Debug, Deserialize)]
struct LegacyConfig {
    claude: AgentCmd,
    codex: AgentCmd,
    lmstudio: LegacyLmConfig,
    params: Params,
}

#[derive(Debug, Deserialize)]
struct LegacyLmConfig {
    base_url: String,
    model: String,
}

impl Default for HttpConfigs {
    fn default() -> Self {
        Self {
            tinker: HttpConfig {
                base_url: "https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1"
                    .to_string(),
                model: "tinker://YOUR_CHECKPOINT".to_string(),
                api_key_env: Some("TINKER_API_KEY".to_string()),
                system_prompt: String::new(),
            },
            lmstudio: HttpConfig {
                base_url: "http://localhost:1234/v1".to_string(),
                model: "local-model".to_string(),
                api_key_env: None,
                system_prompt: String::new(),
            },
        }
    }
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
            http: HttpConfigs {
                tinker: HttpConfig {
                    base_url: "https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1"
                        .to_string(),
                    model: "tinker://YOUR_CHECKPOINT".to_string(),
                    api_key_env: Some("TINKER_API_KEY".to_string()),
                    system_prompt: String::new(),
                },
                lmstudio: HttpConfig {
                    base_url: "http://localhost:1234/v1".to_string(),
                    model: "local-model".to_string(),
                    api_key_env: None,
                    system_prompt: String::new(),
                },
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
        // Migrate legacy [lmstudio]-only config.
        if let Ok(legacy) = toml::from_str::<LegacyConfig>(&text) {
            let mut cfg = Config::default();
            cfg.claude = legacy.claude;
            cfg.codex = legacy.codex;
            cfg.params = legacy.params;
            cfg.http.lmstudio.base_url = legacy.lmstudio.base_url;
            cfg.http.lmstudio.model = legacy.lmstudio.model;
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

    pub fn http_for(&self, provider: crate::session::HttpProvider) -> HttpConfig {
        match provider {
            crate::session::HttpProvider::Tinker => self.http.tinker.clone(),
            crate::session::HttpProvider::LmStudio => self.http.lmstudio.clone(),
        }
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
        assert_eq!(cfg.http.lmstudio.base_url, "http://localhost:1234/v1");
        assert!(cfg.http.tinker.api_key_env.as_deref() == Some("TINKER_API_KEY"));
    }

    #[test]
    fn config_roundtrips_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.params.temperature, cfg.params.temperature);
        assert_eq!(parsed.http.tinker.model, cfg.http.tinker.model);
    }
}
