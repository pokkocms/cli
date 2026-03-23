use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::api::PokkoClient;
use crate::config;
use crate::schema::{FieldType, LocalField, LocalModel, model_to_yaml};
use crate::Cli;

pub async fn run(cli: &Cli, env_override: Option<&str>) -> Result<()> {
    let cfg = config::resolve(
        cli.api_url.as_deref(),
        cli.token.as_deref(),
        cli.project.as_deref(),
        cli.environment.as_deref(),
        &cli.models_dir,
        env_override,
    )?;

    let client = PokkoClient::new(&cfg.api_url, &cfg.token)?;

    println!("Fetching models from {}...", cfg.api_url);

    let remote_models = client
        .list_models(&cfg.project, &cfg.environment)
        .await
        .context("failed to fetch models from API")?;

    if remote_models.is_empty() {
        println!("No models found for this project/environment.");
        return Ok(());
    }

    // Build a UUID → alias map for resolving references
    let id_to_alias: HashMap<String, String> = remote_models
        .iter()
        .map(|m| (m.id.clone(), m.alias.clone()))
        .collect();

    // Ensure models directory exists
    std::fs::create_dir_all(&cfg.models_dir)
        .with_context(|| format!("failed to create models directory: {}", cfg.models_dir.display()))?;

    let mut written = 0usize;

    for remote_model in &remote_models {
        let fields: Vec<LocalField> = remote_model
            .fields
            .iter()
            .map(|rf| remote_field_to_local(rf, &id_to_alias))
            .collect();

        // Resolve inherits UUIDs to aliases
        let inherits: Vec<String> = remote_model
            .inherits
            .iter()
            .map(|id| {
                id_to_alias
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.clone()) // fallback to UUID if not found
            })
            .collect();

        let local_model = LocalModel {
            alias: remote_model.alias.clone(),
            name: remote_model.name.clone(),
            usage: remote_model.usage.to_lowercase(),
            inherits,
            fields,
        };

        let yaml = model_to_yaml(&local_model)
            .with_context(|| format!("failed to serialise model {}", remote_model.alias))?;

        let file_path = cfg.models_dir.join(format!("{}.yml", remote_model.alias));
        std::fs::write(&file_path, yaml)
            .with_context(|| format!("failed to write {}", file_path.display()))?;

        written += 1;
    }

    let models_dir_display = cfg
        .models_dir
        .strip_prefix(&cfg.root)
        .unwrap_or(&cfg.models_dir)
        .display()
        .to_string();

    println!(
        "Pulled {} model{} to {}/",
        written,
        if written == 1 { "" } else { "s" },
        models_dir_display
    );

    Ok(())
}

/// Convert a remote API field into a local YAML field representation.
fn remote_field_to_local(
    rf: &crate::api::RemoteField,
    id_to_alias: &HashMap<String, String>,
) -> LocalField {
    let field_type = match rf.field_type.as_str() {
        "MEDIA" => FieldType::Media,
        "ENTRY" => FieldType::Entry,
        "VALUE" => FieldType::Value,
        "SCALAR" => match rf.scalar_subtype().unwrap_or("text") {
            "richtext" => FieldType::Richtext,
            "date" => FieldType::Date,
            "number" => FieldType::Number,
            "boolean" => FieldType::Boolean,
            _ => FieldType::Text,
        },
        _ => FieldType::Text,
    };

    // Resolve model UUID references to aliases
    let models: Vec<String> = if field_type.supports_models() {
        rf.referenced_model_ids()
            .iter()
            .map(|id| {
                id_to_alias
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.clone())
            })
            .collect()
    } else {
        vec![]
    };

    LocalField {
        alias: rf.alias.clone(),
        name: rf.name.clone(),
        field_type,
        required: rf.required,
        multi: rf.multi,
        models,
    }
}
