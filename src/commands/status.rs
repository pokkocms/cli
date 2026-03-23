use anyhow::{Context, Result};

use crate::api::PokkoClient;
use crate::config;
use crate::diff;
use crate::schema;
use crate::Cli;

use super::plan_cmd::display_only;

pub async fn run(cli: &Cli, env_override: Option<&str>) -> Result<()> {
    let cfg = config::resolve(
        cli.api_url.as_deref(),
        cli.token.as_deref(),
        cli.project.as_deref(),
        cli.environment.as_deref(),
        &cli.models_dir,
        env_override,
    )?;

    let local_models = schema::load_models(&cfg.models_dir)
        .context("failed to load local model files")?;

    let client = PokkoClient::new(&cfg.api_url, &cfg.token)?;
    let remote_models = client
        .list_models(&cfg.project, &cfg.environment)
        .await
        .context("failed to fetch remote models")?;

    let changes = diff::diff(&local_models, &remote_models);
    let summary = diff::summarise(&changes);

    display_only(&changes, &summary);

    Ok(())
}
