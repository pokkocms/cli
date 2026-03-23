use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::api::RemoteModel;
use crate::diff::Change;
use crate::schema::LocalModel;

// ---------------------------------------------------------------------------
// Plan data structure
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub project_id: String,
    pub environment_id: String,
    pub api_url: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub has_destructive: bool,
    pub changes: Vec<Change>,
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Compute a deterministic SHA-256 hash over a sorted, canonical JSON
/// representation of the local models.
pub fn compute_local_hash(models: &[LocalModel]) -> String {
    let mut sorted: Vec<&LocalModel> = models.iter().collect();
    sorted.sort_by(|a, b| a.alias.cmp(&b.alias));

    // Use serde_json's deterministic serialisation (keys are stable in structs)
    let canonical = serde_json::to_string(&sorted).expect("local models must be serialisable");
    sha256_hex(canonical.as_bytes())
}

/// Compute a deterministic SHA-256 hash over a sorted, canonical JSON
/// representation of the remote models.
pub fn compute_remote_hash(models: &[RemoteModel]) -> String {
    let mut sorted: Vec<&RemoteModel> = models.iter().collect();
    sorted.sort_by(|a, b| a.alias.cmp(&b.alias));

    let canonical = serde_json::to_string(&sorted).expect("remote models must be serialisable");
    sha256_hex(canonical.as_bytes())
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

// ---------------------------------------------------------------------------
// Save / Load
// ---------------------------------------------------------------------------

/// Save the plan to `.pokko/plans/plan-<timestamp>-<short_hash>.json` under
/// `root`. Returns the absolute path of the saved file.
pub fn save_plan(plan: &Plan, root: &Path) -> Result<PathBuf> {
    let plans_dir = root.join(".pokko").join("plans");
    std::fs::create_dir_all(&plans_dir)
        .with_context(|| format!("failed to create plans directory: {}", plans_dir.display()))?;

    let ts = plan.created_at.format("%Y%m%dT%H%M%S");

    // Short hash from the local_hash string (take first 8 hex chars after "sha256:")
    let short = plan
        .local_hash
        .strip_prefix("sha256:")
        .unwrap_or(&plan.local_hash)
        .chars()
        .take(8)
        .collect::<String>();

    let filename = format!("plan-{}-{}.json", ts, short);
    let path = plans_dir.join(&filename);

    let content = serde_json::to_string_pretty(plan).context("failed to serialise plan")?;
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write plan file: {}", path.display()))?;

    Ok(path)
}

/// Load a plan from a JSON file.
pub fn load_plan(path: &Path) -> Result<Plan> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read plan file: {}", path.display()))?;

    let plan: Plan =
        serde_json::from_str(&content).with_context(|| format!("invalid plan file: {}", path.display()))?;

    if plan.version != 1 {
        bail!(
            "unsupported plan version {} — this version of pokko only supports version 1",
            plan.version
        );
    }

    Ok(plan)
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Verify that a previously computed plan is still valid by comparing hashes.
///
/// Returns `Ok(())` if both hashes match, or a descriptive error explaining
/// what changed and what the user should do.
pub fn verify_plan(
    plan: &Plan,
    current_local_hash: &str,
    current_remote_hash: &str,
) -> Result<()> {
    let mut errors = Vec::new();

    if plan.local_hash != current_local_hash {
        errors.push(
            "Local model files have changed since the plan was created.\n  \
             The plan may no longer reflect your intended changes.\n  \
             Run `pokko plan` again to generate a new plan."
                .to_string(),
        );
    }

    if plan.remote_hash != current_remote_hash {
        errors.push(
            "Remote state has changed since the plan was created.\n  \
             Another process may have modified the CMS models.\n  \
             Run `pokko plan` again to generate a fresh plan against the current remote state."
                .to_string(),
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("Plan is stale and cannot be applied:\n\n  {}", errors.join("\n\n  "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{FieldType, LocalField, LocalModel};

    fn make_local_model(alias: &str) -> LocalModel {
        LocalModel {
            alias: alias.to_string(),
            name: format!("{} Name", alias),
            usage: "entry".to_string(),
            inherits: vec![],
            fields: vec![LocalField {
                alias: "title".to_string(),
                name: "Title".to_string(),
                field_type: FieldType::Text,
                required: true,
                multi: false,
                models: vec![],
            }],
        }
    }

    #[test]
    fn test_hash_determinism() {
        let models = vec![
            make_local_model("ZebraModel"),
            make_local_model("AlphaModel"),
        ];

        let h1 = compute_local_hash(&models);

        // Reversed order — hash should be the same (sorted internally)
        let models_rev = vec![
            make_local_model("AlphaModel"),
            make_local_model("ZebraModel"),
        ];

        let h2 = compute_local_hash(&models_rev);
        assert_eq!(h1, h2, "hash must be order-independent");
    }

    #[test]
    fn test_hash_changes_on_modification() {
        let models1 = vec![make_local_model("BlogPost")];
        let mut m2 = make_local_model("BlogPost");
        m2.name = "Different Name".to_string();
        let models2 = vec![m2];

        assert_ne!(
            compute_local_hash(&models1),
            compute_local_hash(&models2),
            "hash must differ when model content changes"
        );
    }

    #[test]
    fn test_verify_plan_both_match() {
        let result = verify_plan(
            &Plan {
                version: 1,
                created_at: Utc::now(),
                project_id: "proj".to_string(),
                environment_id: "env".to_string(),
                api_url: "https://au-syd1.pokko.io".to_string(),
                local_hash: "sha256:abc".to_string(),
                remote_hash: "sha256:def".to_string(),
                has_destructive: false,
                changes: vec![],
            },
            "sha256:abc",
            "sha256:def",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_plan_local_mismatch() {
        let result = verify_plan(
            &Plan {
                version: 1,
                created_at: Utc::now(),
                project_id: "proj".to_string(),
                environment_id: "env".to_string(),
                api_url: "https://au-syd1.pokko.io".to_string(),
                local_hash: "sha256:abc".to_string(),
                remote_hash: "sha256:def".to_string(),
                has_destructive: false,
                changes: vec![],
            },
            "sha256:CHANGED",
            "sha256:def",
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Local model files have changed"));
    }
}
