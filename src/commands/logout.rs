use anyhow::Result;
use colored::Colorize;

use crate::credentials;

pub fn run() -> Result<()> {
    match credentials::credentials_path() {
        Some(path) if path.exists() => {
            credentials::delete()?;
            println!("{}", "Logged out. Credentials removed.".green());
        }
        _ => {
            println!("Not currently logged in.");
        }
    }
    Ok(())
}
