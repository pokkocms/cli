use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

use crate::api::{
    CreateFieldInput, CreateModelInput, PokkoClient, UpdateFieldPatch, UpdateModelPatch,
};
use crate::diff::{Change, FieldChange, FieldPropChange};
use crate::plan::{self, compute_local_hash, compute_remote_hash, load_plan};
use crate::schema::{self, FieldType, LocalField, LocalModel};
use crate::Cli;

pub async fn run(cli: &Cli, plan_file: &str, force: bool) -> Result<()> {
    let plan_path = Path::new(plan_file);
    let plan = load_plan(plan_path)
        .with_context(|| format!("failed to load plan from {}", plan_path.display()))?;

    // Override config with plan values where applicable
    let token = cli
        .token
        .clone()
        .or_else(|| std::env::var("POKKO_TOKEN").ok())
        .context("API token not set — set POKKO_TOKEN environment variable")?;

    let client = PokkoClient::new(&plan.api_url, &token)?;

    println!("Verifying plan...");

    // Re-fetch remote state and verify hashes
    let remote_models = client
        .list_models(&plan.project_id, &plan.environment_id)
        .await
        .context("failed to fetch remote models for verification")?;

    let current_remote_hash = compute_remote_hash(&remote_models);

    // We need to load local models to compute the current local hash.
    // Use models_dir from CLI or try to infer from config.
    let local_models = load_local_models(cli)?;
    let current_local_hash = compute_local_hash(&local_models);

    plan::verify_plan(&plan, &current_local_hash, &current_remote_hash)
        .context("plan verification failed")?;

    println!("{}", "Plan verified.".green());

    // Check destructive flag
    if plan.has_destructive && !force {
        bail!(
            "This plan contains destructive operations (deletions).\n  \
             To apply destructive changes, re-run with --force:\n  \
             \n  \
               pokko apply {} --force\n  \
             \n  \
             Review the plan carefully before applying with --force.",
            plan_file
        );
    }

    if plan.changes.is_empty() {
        println!("Nothing to apply — no changes in plan.");
        return Ok(());
    }

    println!();
    println!("Applying {} change(s)...", plan.changes.len());
    println!();

    // Build a mutable alias → ID map, seeded from remote state.
    // This is updated as we create new models so cross-references resolve correctly.
    let mut alias_to_id: HashMap<String, String> = remote_models
        .iter()
        .map(|m| (m.alias.clone(), m.id.clone()))
        .collect();

    // We process in a specific order for safety:
    //  1. CreateModel  (topologically ordered)
    //  2. UpdateModel  (metadata only)
    //  3. Fields (create/update/reorder)
    //  4. DeleteField  (only if --force)
    //  5. DeleteModel  (only if --force)

    let (creates, updates, deletes) = partition_changes(&plan.changes);

    // --- Step 1: Create models in topological order ---
    let ordered_creates = topological_sort_creates(&creates);
    for model in &ordered_creates {
        print!("  {} Creating model {}...", "+".green(), model.alias);

        // Resolve inherits aliases to IDs
        let inherits_ids: Vec<String> = resolve_aliases(&model.inherits, &alias_to_id)?;

        let input = CreateModelInput {
            project_id: plan.project_id.clone(),
            environment_id: plan.environment_id.clone(),
            alias: model.alias.clone(),
            name: model.name.clone(),
            usage: model.usage.to_uppercase(),
            inherits: inherits_ids,
        };

        let new_id = client
            .create_model(&input)
            .await
            .with_context(|| format!("failed to create model {}", model.alias))?;

        alias_to_id.insert(model.alias.clone(), new_id.clone());
        println!(" {}", format!("done ({})", &new_id[..8.min(new_id.len())]).dimmed());

        // Create fields for the new model
        create_fields_for_model(
            &client,
            &new_id,
            &model.fields,
            &plan.project_id,
            &plan.environment_id,
            &alias_to_id,
        )
        .await
        .with_context(|| format!("failed to create fields for model {}", model.alias))?;
    }

    // --- Step 2 & 3: Update models and their fields ---
    for update in &updates {
        let Change::UpdateModel {
            id,
            alias,
            name_change,
            usage_change,
            inherits_change,
            field_changes,
        } = update else { continue };

        let has_model_changes =
            name_change.is_some() || usage_change.is_some() || inherits_change.is_some();

        if has_model_changes {
            print!("  {} Updating model {}...", "~".yellow(), alias);

            let new_inherits = if let Some((_, new_aliases)) = inherits_change {
                Some(resolve_aliases(new_aliases, &alias_to_id)?)
            } else {
                None
            };

            let patch = UpdateModelPatch {
                name: name_change.as_ref().map(|(_, new)| new.clone()),
                usage: usage_change.as_ref().map(|(_, new)| new.to_uppercase()),
                inherits: new_inherits,
            };

            client
                .update_model(id, &patch)
                .await
                .with_context(|| format!("failed to update model {}", alias))?;

            println!(" {}", "done".dimmed());
        }

        // Process field changes in a single pass: non-destructive always, destructive only with --force
        for fc in field_changes {
            if let FieldChange::DeleteField {
                id: field_id,
                alias: field_alias,
                ..
            } = fc
            {
                if force {
                    print!(
                        "  {} Deleting field {}.{}...",
                        "-".red(),
                        alias,
                        field_alias
                    );
                    client
                        .soft_delete_field(field_id)
                        .await
                        .with_context(|| {
                            format!("failed to delete field {}.{}", alias, field_alias)
                        })?;
                    println!(" {}", "done".dimmed());
                }
            } else {
                apply_field_change_non_destructive(
                    &client,
                    fc,
                    id,
                    alias,
                    &plan.project_id,
                    &plan.environment_id,
                    &alias_to_id,
                    &remote_models,
                )
                .await?;
            }
        }
    }

    // --- Step 4 & 5: Destructive deletes (--force only) ---
    if force {
        for delete in &deletes {
            if let Change::DeleteModel { id, alias, .. } = delete {
                print!("  {} Deleting model {}...", "-".red(), alias);
                client
                    .soft_delete_model(id)
                    .await
                    .with_context(|| format!("failed to delete model {}", alias))?;
                println!(" {}", "done".dimmed());
            }
        }
    }

    println!();
    println!("{}", "Apply complete.".green().bold());

    // Summary
    let created = ordered_creates.len();
    let updated = updates.len();
    let deleted = if force { deletes.len() } else { 0 };
    println!(
        "  {} created, {} updated, {} destroyed.",
        created, updated, deleted
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn partition_changes(changes: &[Change]) -> (Vec<LocalModel>, Vec<&Change>, Vec<&Change>) {
    let mut creates = Vec::new();
    let mut updates = Vec::new();
    let mut deletes = Vec::new();

    for c in changes {
        match c {
            Change::CreateModel { model } => creates.push(model.clone()),
            Change::UpdateModel { .. } => updates.push(c),
            Change::DeleteModel { .. } => deletes.push(c),
        }
    }

    (creates, updates, deletes)
}

/// Very simple topological sort: models that inherit from others go after them.
fn topological_sort_creates(creates: &[LocalModel]) -> Vec<LocalModel> {
    let alias_set: std::collections::HashSet<String> =
        creates.iter().map(|m| m.alias.clone()).collect();

    let mut result: Vec<LocalModel> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Build adjacency: model alias -> list of inherited aliases (within this create batch)
    let deps: HashMap<String, Vec<String>> = creates
        .iter()
        .map(|m| {
            let deps: Vec<String> = m
                .inherits
                .iter()
                .filter(|a| alias_set.contains(*a))
                .cloned()
                .collect();
            (m.alias.clone(), deps)
        })
        .collect();

    fn visit(
        alias: &str,
        deps: &HashMap<String, Vec<String>>,
        creates: &[LocalModel],
        visited: &mut std::collections::HashSet<String>,
        result: &mut Vec<LocalModel>,
    ) {
        if visited.contains(alias) {
            return;
        }
        visited.insert(alias.to_string());
        if let Some(parent_aliases) = deps.get(alias) {
            for parent in parent_aliases {
                visit(parent, deps, creates, visited, result);
            }
        }
        if let Some(model) = creates.iter().find(|m| m.alias == alias) {
            result.push(model.clone());
        }
    }

    for model in creates {
        visit(
            &model.alias,
            &deps,
            creates,
            &mut visited,
            &mut result,
        );
    }

    result
}

fn resolve_aliases(aliases: &[String], alias_to_id: &HashMap<String, String>) -> Result<Vec<String>> {
    aliases
        .iter()
        .map(|alias| {
            alias_to_id
                .get(alias)
                .cloned()
                .with_context(|| {
                    format!(
                        "cannot resolve alias '{}' to a model ID — make sure this model exists \
                         or is being created in the same plan",
                        alias
                    )
                })
        })
        .collect()
}

/// Build the API config JSON for a local field.
fn build_field_config(
    field: &LocalField,
    alias_to_id: &HashMap<String, String>,
) -> Result<serde_json::Value> {
    match &field.field_type {
        FieldType::Text => Ok(json!({ "type": "text" })),
        FieldType::Richtext => Ok(json!({ "type": "richtext" })),
        FieldType::Date => Ok(json!({ "type": "date" })),
        FieldType::Number => Ok(json!({ "type": "number" })),
        FieldType::Boolean => Ok(json!({ "type": "boolean" })),
        FieldType::Media => Ok(json!({})),
        FieldType::Entry | FieldType::Value => {
            let model_ids = resolve_aliases(&field.models, alias_to_id)?;
            Ok(json!({ "models": model_ids }))
        }
    }
}

async fn create_fields_for_model(
    client: &PokkoClient,
    model_id: &str,
    fields: &[LocalField],
    project_id: &str,
    environment_id: &str,
    alias_to_id: &HashMap<String, String>,
) -> Result<()> {
    for (idx, field) in fields.iter().enumerate() {
        print!(
            "    {} Creating field {}...",
            "+".green(),
            field.alias
        );

        let config = build_field_config(field, alias_to_id)
            .with_context(|| format!("failed to build config for field {}", field.alias))?;

        let input = CreateFieldInput {
            model_id: model_id.to_string(),
            project_id: project_id.to_string(),
            environment_id: environment_id.to_string(),
            alias: field.alias.clone(),
            name: field.name.clone(),
            field_type: field.field_type.to_api_type().to_string(),
            config,
            required: field.required,
            multi: field.multi,
            index: idx as i64,
        };

        client.create_field(&input).await.with_context(|| {
            format!("failed to create field {}", field.alias)
        })?;

        println!(" {}", "done".dimmed());
    }
    Ok(())
}

async fn apply_field_change_non_destructive(
    client: &PokkoClient,
    fc: &FieldChange,
    model_id: &str,
    model_alias: &str,
    project_id: &str,
    environment_id: &str,
    alias_to_id: &HashMap<String, String>,
    remote_models: &[crate::api::RemoteModel],
) -> Result<()> {
    match fc {
        FieldChange::CreateField { field } => {
            // Determine index: count existing remote fields for this model
            let remote_model = remote_models.iter().find(|m| m.id == model_id);
            let base_index = remote_model.map(|m| m.fields.len()).unwrap_or(0);

            print!(
                "    {} Creating field {}.{}...",
                "+".green(),
                model_alias,
                field.alias
            );

            let config = build_field_config(field, alias_to_id)?;

            let input = CreateFieldInput {
                model_id: model_id.to_string(),
                project_id: project_id.to_string(),
                environment_id: environment_id.to_string(),
                alias: field.alias.clone(),
                name: field.name.clone(),
                field_type: field.field_type.to_api_type().to_string(),
                config,
                required: field.required,
                multi: field.multi,
                index: base_index as i64,
            };

            client
                .create_field(&input)
                .await
                .with_context(|| format!("failed to create field {}", field.alias))?;

            println!(" {}", "done".dimmed());
        }

        FieldChange::UpdateField {
            id,
            alias,
            changes,
        } => {
            print!(
                "    {} Updating field {}.{}...",
                "~".yellow(),
                model_alias,
                alias
            );

            let mut patch = UpdateFieldPatch {
                name: None,
                field_type: None,
                config: None,
                required: None,
                multi: None,
                index: None,
            };

            for prop in changes {
                match prop {
                    FieldPropChange::Name(_, new) => patch.name = Some(new.clone()),
                    FieldPropChange::Type(_, new) => {
                        // Map local type string back to API type
                        let ft: FieldType = match new.as_str() {
                            "text" => FieldType::Text,
                            "richtext" => FieldType::Richtext,
                            "date" => FieldType::Date,
                            "number" => FieldType::Number,
                            "boolean" => FieldType::Boolean,
                            "media" => FieldType::Media,
                            "entry" => FieldType::Entry,
                            "value" => FieldType::Value,
                            other => anyhow::bail!("unknown field type: {}", other),
                        };
                        patch.field_type = Some(ft.to_api_type().to_string());
                        // Also update config to match new type if it's a scalar
                        if let Some(subtype) = ft.scalar_subtype() {
                            patch.config = Some(json!({ "type": subtype }));
                        }
                    }
                    FieldPropChange::Required(_, new) => patch.required = Some(*new),
                    FieldPropChange::Multi(_, new) => patch.multi = Some(*new),
                    FieldPropChange::Models(_, new_aliases) => {
                        let model_ids = resolve_aliases(new_aliases, alias_to_id)?;
                        patch.config = Some(json!({ "models": model_ids }));
                    }
                }
            }

            client
                .update_field(id, &patch)
                .await
                .with_context(|| format!("failed to update field {}", alias))?;

            println!(" {}", "done".dimmed());
        }

        FieldChange::ReorderFields { order } => {
            print!(
                "    {} Reordering fields for {}...",
                "~".yellow(),
                model_alias
            );

            let remote_model = remote_models.iter().find(|m| m.id == model_id);
            if let Some(rm) = remote_model {
                let ordered: Vec<(String, i64)> = order
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, alias)| {
                        rm.fields
                            .iter()
                            .find(|f| &f.alias == alias)
                            .map(|f| (f.id.clone(), idx as i64))
                    })
                    .collect();

                client
                    .reorder_fields(&ordered)
                    .await
                    .with_context(|| format!("failed to reorder fields for {}", model_alias))?;
            }

            println!(" {}", "done".dimmed());
        }

        // DeleteField is handled by the caller under --force
        FieldChange::DeleteField { .. } => {}
    }

    Ok(())
}

fn load_local_models(cli: &Cli) -> Result<Vec<crate::schema::LocalModel>> {
    // Try to resolve config to get models_dir; fall back to CLI default
    let models_dir = match crate::config::load_config() {
        Ok((cfg, root)) => {
            let dir_str = if cli.models_dir != "models" {
                cli.models_dir.clone()
            } else {
                cfg.models_dir
            };
            if std::path::Path::new(&dir_str).is_absolute() {
                std::path::PathBuf::from(&dir_str)
            } else {
                root.join(&dir_str)
            }
        }
        Err(_) => {
            let cwd = std::env::current_dir()?;
            cwd.join(&cli.models_dir)
        }
    };

    schema::load_models(&models_dir)
        .with_context(|| format!("failed to load models from {}", models_dir.display()))
}
