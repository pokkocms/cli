use anyhow::Result;
use colored::Colorize;

use crate::credentials;

pub fn run() -> Result<()> {
    match credentials::load() {
        Some(creds) => {
            println!("{}", "Authenticated".green().bold());
            println!("  Auth URL: {}", creds.auth_url);
            // Print a truncated token so the user can verify it without exposing the full secret
            let preview = if creds.token.len() > 20 {
                format!("{}…", &creds.token[..20])
            } else {
                creds.token.clone()
            };
            println!("  Token:    {}", preview);
            println!();
            println!("To log out, run: pokko logout");
        }
        None => {
            println!("{}", "Not logged in.".yellow());
            println!("Run `pokko login` to authenticate.");
        }
    }
    Ok(())
}
