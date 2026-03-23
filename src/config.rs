use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::credentials;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_url: String,
    pub project: String,
    pub environment: String,
    pub models_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_url: "https://au-syd1.pokko.io/graphql".to_string(),
            project: String::new(),
            environment: String::new(),
            models_dir: "models".to_string(),
        }
    }
}

/// Walk up the directory tree from `start` to find a `.pokko/config.toml`.
/// Returns the path to the config file and the root directory containing `.pokko/`.
pub fn find_config_file(start: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(".pokko").join("config.toml");
        if candidate.exists() {
            return Some((candidate, current));
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Load config from disk, walking up to find `.pokko/config.toml`.
pub fn load_config() -> Result<(Config, PathBuf)> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let (config_path, root) =
        find_config_file(&cwd).context("no .pokko/config.toml found — run `pokko init` first")?;

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;

    let config: Config =
        toml::from_str(&content).with_context(|| format!("invalid config at {}", config_path.display()))?;

    Ok((config, root))
}

/// Save config to `.pokko/config.toml` relative to `root`.
pub fn save_config(config: &Config, root: &Path) -> Result<()> {
    let pokko_dir = root.join(".pokko");
    std::fs::create_dir_all(&pokko_dir)
        .with_context(|| format!("failed to create directory {}", pokko_dir.display()))?;

    let config_path = pokko_dir.join("config.toml");
    let content = toml::to_string_pretty(config).context("failed to serialize config")?;
    std::fs::write(&config_path, content)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(())
}

/// Resolve effective config by merging CLI flags over loaded file config.
/// Precedence: CLI flag > env var (handled by clap) > config file > default.
pub fn resolve(
    cli_api_url: Option<&str>,
    cli_token: Option<&str>,
    cli_project: Option<&str>,
    cli_environment: Option<&str>,
    cli_models_dir: &str,
    env_override: Option<&str>,
) -> Result<ResolvedConfig> {
    // Try to load file config; if not found we'll rely on CLI flags entirely.
    let (file_config, root) = match load_config() {
        Ok(pair) => pair,
        Err(_) => {
            // No config file — all values must come from CLI/env
            let api_url = cli_api_url
                .unwrap_or("https://au-syd1.pokko.io/graphql")
                .to_string();
            let token = cli_token
                .map(|s| s.to_string())
                .or_else(|| std::env::var("POKKO_TOKEN").ok())
                .or_else(|| credentials::load().map(|c| c.token));
            let project = cli_project
                .map(|s| s.to_string())
                .or_else(|| std::env::var("POKKO_PROJECT").ok());
            let environment = env_override
                .or(cli_environment)
                .map(|s| s.to_string())
                .or_else(|| std::env::var("POKKO_ENVIRONMENT").ok());

            let project = project.context(
                "project UUID not set — use --project, POKKO_PROJECT env var, or run `pokko init`",
            )?;
            let environment = environment.context(
                "environment UUID not set — use --environment, POKKO_ENVIRONMENT env var, or run `pokko init`",
            )?;
            let token = token.context(
                "not authenticated — run `pokko login` or set POKKO_TOKEN",
            )?;

            let cwd = std::env::current_dir().context("failed to get current directory")?;
            return Ok(ResolvedConfig {
                api_url,
                token,
                project,
                environment,
                models_dir: cwd.join(cli_models_dir),
                root: cwd,
            });
        }
    };

    let api_url = cli_api_url
        .unwrap_or(&file_config.api_url)
        .to_string();

    let token = cli_token
        .map(|s| s.to_string())
        .or_else(|| std::env::var("POKKO_TOKEN").ok())
        .or_else(|| credentials::load().map(|c| c.token))
        .context("not authenticated — run `pokko login` or set POKKO_TOKEN")?;

    let project = cli_project
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_config.project.clone());

    let environment = env_override
        .or(cli_environment)
        .map(|s| s.to_string())
        .unwrap_or_else(|| file_config.environment.clone());

    if project.is_empty() {
        bail!("project UUID not set — update .pokko/config.toml or use --project");
    }
    if environment.is_empty() {
        bail!("environment UUID not set — update .pokko/config.toml or use --environment");
    }

    // models_dir: if CLI default ("models") was passed, prefer file config
    let models_dir_str = if cli_models_dir == "models" {
        file_config.models_dir.clone()
    } else {
        cli_models_dir.to_string()
    };

    let models_dir = if std::path::Path::new(&models_dir_str).is_absolute() {
        PathBuf::from(&models_dir_str)
    } else {
        root.join(&models_dir_str)
    };

    Ok(ResolvedConfig {
        api_url,
        token,
        project,
        environment,
        models_dir,
        root,
    })
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub api_url: String,
    pub token: String,
    pub project: String,
    pub environment: String,
    pub models_dir: PathBuf,
    pub root: PathBuf,
}
