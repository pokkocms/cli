use anyhow::{Context, Result};
use colored::Colorize;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::api::{PokkoClient, RemoteEnvironment, RemoteProject};
use crate::config::{save_config, Config};
use crate::credentials;
use crate::Cli;

pub async fn run(cli: &Cli) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let pokko_dir = cwd.join(".pokko");
    let config_path = pokko_dir.join("config.toml");

    if config_path.exists() {
        println!(
            "{} .pokko/config.toml already exists.",
            "Warning:".yellow().bold()
        );
        print!("Overwrite? [y/N]: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
        println!();
    }

    println!("{}", "Initialising Pokko CLI...".bold());
    println!();

    // ── API URL ──────────────────────────────────────────────────────────────
    let api_url = prompt_with_default(
        "API URL",
        cli.api_url
            .as_deref()
            .unwrap_or("https://au-syd1.pokko.io/graphql"),
    )?;

    // ── Resolve token (for fetching projects) ─────────────────────────────
    let token = cli
        .token
        .clone()
        .or_else(|| std::env::var("POKKO_TOKEN").ok())
        .or_else(|| credentials::load().map(|c| c.token));

    // ── Project selection ─────────────────────────────────────────────────
    let (project_id, environments) = match &token {
        Some(tok) => {
            select_project_interactive(&api_url, tok, cli.project.as_deref()).await?
        }
        None => {
            println!(
                "{} Not authenticated — run {} to enable interactive project selection.",
                "Note:".yellow(),
                "`pokko login`".cyan()
            );
            println!();
            let id = prompt_required("Project UUID", cli.project.as_deref())?;
            (id, vec![])
        }
    };

    // ── Environment selection ─────────────────────────────────────────────
    let environment_id = match &token {
        Some(_) if !environments.is_empty() => {
            select_environment_interactive(&environments, cli.environment.as_deref())?
        }
        _ => prompt_required("Environment UUID", cli.environment.as_deref())?,
    };

    // ── Models directory ──────────────────────────────────────────────────
    let models_dir = prompt_with_default("Models directory", &cli.models_dir)?;

    // ── Write config ──────────────────────────────────────────────────────
    println!();
    let config = Config {
        api_url,
        project: project_id,
        environment: environment_id,
        models_dir: models_dir.clone(),
    };
    save_config(&config, &cwd)?;
    println!(
        "  {} .pokko/config.toml",
        "Created".green()
    );

    // Create models dir
    let models_path: PathBuf = if std::path::Path::new(&models_dir).is_absolute() {
        PathBuf::from(&models_dir)
    } else {
        cwd.join(&models_dir)
    };
    if !models_path.exists() {
        std::fs::create_dir_all(&models_path)?;
        println!(
            "  {} {}/",
            "Created".green(),
            models_path.strip_prefix(&cwd).unwrap_or(&models_path).display()
        );
    }

    // Create .pokko/plans dir
    let plans_dir = pokko_dir.join("plans");
    std::fs::create_dir_all(&plans_dir)?;
    println!("  {} .pokko/plans/", "Created".green());

    println!();
    println!("{}", "Pokko CLI initialised successfully.".green().bold());
    println!();
    println!("Next steps:");
    println!("  pokko pull        # fetch current model schemas");
    println!("  pokko plan        # preview any pending changes");
    println!("  pokko apply <plan> # apply a saved plan");

    Ok(())
}

// ── Interactive project selection ─────────────────────────────────────────────

async fn select_project_interactive(
    api_url: &str,
    token: &str,
    preselected: Option<&str>,
) -> Result<(String, Vec<RemoteEnvironment>)> {
    let client = PokkoClient::new(api_url, token)?;

    print!("Fetching projects...");
    io::stdout().flush()?;

    let projects = match client.list_projects().await {
        Ok(p) => {
            println!(" done.");
            p
        }
        Err(e) => {
            println!(" failed.");
            println!(
                "{} Could not fetch projects: {}",
                "Warning:".yellow(),
                e
            );
            println!();
            let id = prompt_required("Project UUID", preselected)?;
            return Ok((id, vec![]));
        }
    };

    if projects.is_empty() {
        println!("{} No projects found for this account.", "Warning:".yellow());
        let id = prompt_required("Project UUID", preselected)?;
        return Ok((id, vec![]));
    }

    // If --project was given as a UUID, find it in the list
    if let Some(pre) = preselected {
        if let Some(proj) = projects.iter().find(|p| p.id == pre || p.key == pre) {
            println!("Using project: {} ({})", proj.name.bold(), proj.key);
            return Ok((proj.id.clone(), proj.environments.clone()));
        }
        // Not found — fall through to interactive picker but show a warning
        println!(
            "{} Project '{}' not found in the list; select below.",
            "Note:".yellow(),
            pre
        );
    }

    let chosen = pick_project(&projects)?;
    Ok((chosen.id.clone(), chosen.environments.clone()))
}

fn pick_project(projects: &[RemoteProject]) -> Result<&RemoteProject> {
    println!();
    println!("{}:", "Available projects".bold());
    println!();
    for (i, p) in projects.iter().enumerate() {
        println!(
            "  {}. {} {}  — {}",
            (i + 1).to_string().cyan(),
            p.name.bold(),
            format!("({})", p.key).dimmed(),
            p.account.name.dimmed()
        );
    }
    println!();

    loop {
        print!("Select project [1]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        let idx: usize = if trimmed.is_empty() {
            1
        } else {
            match trimmed.parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    println!("  Enter a number between 1 and {}.", projects.len());
                    continue;
                }
            }
        };

        if idx == 0 || idx > projects.len() {
            println!("  Enter a number between 1 and {}.", projects.len());
            continue;
        }

        return Ok(&projects[idx - 1]);
    }
}

// ── Interactive environment selection ────────────────────────────────────────

fn select_environment_interactive(
    environments: &[RemoteEnvironment],
    preselected: Option<&str>,
) -> Result<String> {
    if environments.is_empty() {
        return prompt_required("Environment UUID", preselected);
    }

    // If --environment was given, find it
    if let Some(pre) = preselected {
        if let Some(env) = environments.iter().find(|e| e.id == pre || e.key == pre) {
            println!("Using environment: {} ({})", env.name.bold(), env.key);
            return Ok(env.id.clone());
        }
        println!(
            "{} Environment '{}' not found; select below.",
            "Note:".yellow(),
            pre
        );
    }

    println!();
    println!("{}:", "Available environments".bold());
    println!();
    for (i, e) in environments.iter().enumerate() {
        println!(
            "  {}. {} {}",
            (i + 1).to_string().cyan(),
            e.name.bold(),
            format!("({})", e.key).dimmed()
        );
    }
    println!();

    loop {
        print!("Select environment [1]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        let idx: usize = if trimmed.is_empty() {
            1
        } else {
            match trimmed.parse::<usize>() {
                Ok(n) => n,
                Err(_) => {
                    println!("  Enter a number between 1 and {}.", environments.len());
                    continue;
                }
            }
        };

        if idx == 0 || idx > environments.len() {
            println!("  Enter a number between 1 and {}.", environments.len());
            continue;
        }

        return Ok(environments[idx - 1].id.clone());
    }
}

// ── Shared prompt helpers ─────────────────────────────────────────────────────

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", label, default);
    io::stdout().flush().context("failed to flush stdout")?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).context("failed to read input")?;

    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

fn prompt_required(label: &str, default: Option<&str>) -> Result<String> {
    loop {
        if let Some(d) = default {
            print!("{} [{}]: ", label, d);
        } else {
            print!("{}: ", label);
        }
        io::stdout().flush().context("failed to flush stdout")?;

        let mut input = String::new();
        io::stdin().read_line(&mut input).context("failed to read input")?;

        let trimmed = input.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        if let Some(d) = default {
            if !d.is_empty() {
                return Ok(d.to_string());
            }
        }

        println!("  This value is required.");
    }
}
