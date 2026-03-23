use anyhow::{Context, Result};

use crate::api::PokkoClient;
use crate::config;
use crate::diff::{self, Change, FieldChange, FieldPropChange};
use crate::plan::{self, Plan};
use crate::schema;
use crate::Cli;
use colored::Colorize;

pub async fn run(cli: &Cli, env_override: Option<&str>, out_override: Option<&str>) -> Result<()> {
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

    let local_hash = plan::compute_local_hash(&local_models);
    let remote_hash = plan::compute_remote_hash(&remote_models);

    let changes = diff::diff(&local_models, &remote_models);

    print_changes(&changes);

    let summary = diff::summarise(&changes);
    println!();

    if changes.is_empty() {
        println!("{}", "No changes. Models are up to date.".bold());
        return Ok(());
    }

    println!(
        "{}",
        format!(
            "Plan: {} to add, {} to change, {} to destroy.",
            summary.to_add, summary.to_change, summary.to_destroy
        )
        .bold()
    );

    let has_destructive = diff::has_destructive_changes(&changes);

    if has_destructive {
        println!();
        println!(
            "{}",
            "Warning: Plan has destructive operations (deletions).".yellow().bold()
        );
        println!(
            "{}",
            "         Pass --force to `pokko apply` to execute destructive changes.".yellow()
        );
    }

    let pokko_plan = Plan {
        version: 1,
        created_at: chrono::Utc::now(),
        project_id: cfg.project.clone(),
        environment_id: cfg.environment.clone(),
        api_url: cfg.api_url.clone(),
        local_hash,
        remote_hash,
        has_destructive,
        changes,
    };

    let save_root = if let Some(out_path) = out_override {
        // Save to the explicit output path
        let path = std::path::Path::new(out_path);
        let content =
            serde_json::to_string_pretty(&pokko_plan).context("failed to serialise plan")?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create directory: {}", parent.display()))?;
            }
        }
        std::fs::write(path, content)
            .with_context(|| format!("failed to write plan to {}", path.display()))?;
        println!();
        println!("Saved plan to: {}", path.display());
        println!("To apply: pokko apply {}", path.display());
        return Ok(());
    } else {
        cfg.root.clone()
    };

    let plan_path = plan::save_plan(&pokko_plan, &save_root)
        .context("failed to save plan file")?;

    let display_path = plan_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&plan_path);

    println!();
    println!("Saved plan to: {}", display_path.display());
    println!("To apply:      pokko apply {}", display_path.display());

    Ok(())
}

/// Print all changes in a Terraform-style coloured diff.
pub fn print_changes(changes: &[Change]) {
    if changes.is_empty() {
        return;
    }

    println!();

    for change in changes {
        match change {
            Change::CreateModel { model } => {
                println!(
                    "{}  model.{}",
                    "+ create".green().bold(),
                    model.alias
                );
                println!("      name:   {}", model.name);
                println!("      usage:  {}", model.usage);
                if !model.inherits.is_empty() {
                    println!("      inherits: [{}]", model.inherits.join(", "));
                }
                for field in &model.fields {
                    println!(
                        "      {} field.{} ({})",
                        "+".green(),
                        field.alias,
                        field.field_type
                    );
                }
            }

            Change::DeleteModel { alias, name, .. } => {
                println!(
                    "{}  model.{}  {} [DESTRUCTIVE]",
                    "- delete".red().bold(),
                    alias,
                    name.dimmed()
                );
            }

            Change::UpdateModel {
                alias,
                name_change,
                usage_change,
                inherits_change,
                field_changes,
                ..
            } => {
                println!("{}  model.{}", "~ update".yellow().bold(), alias);

                if let Some((old, new)) = name_change {
                    println!(
                        "      name:   {} -> {}",
                        old.dimmed(),
                        new.yellow()
                    );
                }

                if let Some((old, new)) = usage_change {
                    println!(
                        "      usage:  {} -> {}",
                        old.dimmed(),
                        new.yellow()
                    );
                }

                if let Some((old, new)) = inherits_change {
                    println!(
                        "      inherits: [{}] -> [{}]",
                        old.join(", ").dimmed(),
                        new.join(", ").yellow()
                    );
                }

                print_field_changes(field_changes);
            }
        }
    }
}

fn print_field_changes(field_changes: &[FieldChange]) {
    for fc in field_changes {
        match fc {
            FieldChange::CreateField { field } => {
                println!(
                    "      {}  field.{} ({})",
                    "+ create".green(),
                    field.alias,
                    field.field_type
                );
                if field.required {
                    println!("          required: true");
                }
                if field.multi {
                    println!("          multi: true");
                }
                if !field.models.is_empty() {
                    println!("          models: [{}]", field.models.join(", "));
                }
            }

            FieldChange::DeleteField { alias, name, .. } => {
                println!(
                    "      {}  field.{}  {}  {}",
                    "- delete".red(),
                    alias,
                    name.dimmed(),
                    "[DESTRUCTIVE]".red()
                );
            }

            FieldChange::UpdateField { alias, changes, .. } => {
                println!("      {}  field.{}", "~ update".yellow(), alias);
                for prop in changes {
                    match prop {
                        FieldPropChange::Name(old, new) => {
                            println!(
                                "          name:     {} -> {}",
                                old.dimmed(),
                                new.yellow()
                            );
                        }
                        FieldPropChange::Type(old, new) => {
                            println!(
                                "          type:     {} -> {}",
                                old.dimmed(),
                                new.yellow()
                            );
                        }
                        FieldPropChange::Required(old, new) => {
                            println!(
                                "          required: {} -> {}",
                                old.to_string().dimmed(),
                                new.to_string().yellow()
                            );
                        }
                        FieldPropChange::Multi(old, new) => {
                            println!(
                                "          multi:    {} -> {}",
                                old.to_string().dimmed(),
                                new.to_string().yellow()
                            );
                        }
                        FieldPropChange::Models(old, new) => {
                            println!(
                                "          models:   [{}] -> [{}]",
                                old.join(", ").dimmed(),
                                new.join(", ").yellow()
                            );
                        }
                    }
                }
            }

            FieldChange::ReorderFields { order } => {
                println!(
                    "      {}  fields: [{}]",
                    "~ reorder".yellow(),
                    order.join(", ")
                );
            }
        }
    }
}

/// Display changes to stdout without saving a plan (used by `status`).
pub fn display_only(
    changes: &[Change],
    summary: &diff::ChangeSummary,
) {
    print_changes(changes);
    println!();

    if changes.is_empty() {
        println!("{}", "No changes. Models are up to date.".bold());
    } else {
        println!(
            "{}",
            format!(
                "Plan: {} to add, {} to change, {} to destroy.",
                summary.to_add, summary.to_change, summary.to_destroy
            )
            .bold()
        );
    }
}
