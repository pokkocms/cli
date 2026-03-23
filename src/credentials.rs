use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub token: String,
    pub auth_url: String,
}

pub fn credentials_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".pokko").join("credentials.toml"))
}

pub fn load() -> Option<Credentials> {
    let path = credentials_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

pub fn save(creds: &Credentials) -> Result<()> {
    let path = credentials_path().context("cannot determine home directory")?;
    std::fs::create_dir_all(path.parent().unwrap())
        .context("failed to create ~/.pokko directory")?;
    let content = toml::to_string_pretty(creds).context("failed to serialize credentials")?;
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

pub fn delete() -> Result<()> {
    let path = credentials_path().context("cannot determine home directory")?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to remove {}", path.display())),
    }
}
