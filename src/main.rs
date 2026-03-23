use anyhow::Result;
use clap::{Parser, Subcommand};

mod api;
mod commands;
mod config;
mod credentials;
mod diff;
mod plan;
mod schema;

#[derive(Parser)]
#[command(name = "pokko")]
#[command(about = "Manage Pokko CMS model schemas as code", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Pokko API URL
    #[arg(long, env = "POKKO_API_URL", global = true)]
    pub api_url: Option<String>,

    /// Pokko API token
    #[arg(long, env = "POKKO_TOKEN", global = true)]
    pub token: Option<String>,

    /// Project UUID
    #[arg(long, env = "POKKO_PROJECT", global = true)]
    pub project: Option<String>,

    /// Environment UUID
    #[arg(long, env = "POKKO_ENVIRONMENT", global = true)]
    pub environment: Option<String>,

    /// Path to models directory
    #[arg(long, default_value = "models", global = true)]
    pub models_dir: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scaffold .pokko/config.toml and create models/ directory
    Init,

    /// Fetch remote state and write YAML files to models/
    Pull {
        /// Environment UUID (overrides config/flag)
        #[arg(long)]
        env: Option<String>,
    },

    /// Diff local vs remote and save a plan file
    Plan {
        /// Environment UUID (overrides config/flag)
        #[arg(long)]
        env: Option<String>,

        /// Path to save the plan file
        #[arg(long)]
        out: Option<String>,
    },

    /// Apply a previously saved plan
    Apply {
        /// Path to the plan file
        plan_file: String,

        /// Apply even if plan has destructive changes
        #[arg(long)]
        force: bool,
    },

    /// Show current diff without saving a plan
    Status {
        /// Environment UUID (overrides config/flag)
        #[arg(long)]
        env: Option<String>,
    },

    /// Authenticate with Pokko via browser
    Login {
        /// Auth service URL (default: https://id.pokko.io)
        #[arg(long, env = "POKKO_AUTH_URL", default_value = "https://id.pokko.io")]
        auth_url: String,
    },

    /// Remove stored credentials
    Logout,

    /// Show current authentication status
    Whoami,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Init => commands::init::run(&cli).await,
        Commands::Pull { env } => commands::pull::run(&cli, env.as_deref()).await,
        Commands::Plan { env, out } => {
            commands::plan_cmd::run(&cli, env.as_deref(), out.as_deref()).await
        }
        Commands::Apply { plan_file, force } => {
            commands::apply::run(&cli, plan_file, *force).await
        }
        Commands::Status { env } => commands::status::run(&cli, env.as_deref()).await,
        Commands::Login { auth_url } => commands::login::run(auth_url).await,
        Commands::Logout => commands::logout::run(),
        Commands::Whoami => commands::whoami::run(),
    }
}
