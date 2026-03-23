use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Supported field types in local YAML schema files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Richtext,
    Date,
    Number,
    Boolean,
    Media,
    Entry,
    Value,
}

impl std::fmt::Display for FieldType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldType::Text => write!(f, "text"),
            FieldType::Richtext => write!(f, "richtext"),
            FieldType::Date => write!(f, "date"),
            FieldType::Number => write!(f, "number"),
            FieldType::Boolean => write!(f, "boolean"),
            FieldType::Media => write!(f, "media"),
            FieldType::Entry => write!(f, "entry"),
            FieldType::Value => write!(f, "value"),
        }
    }
}

impl FieldType {
    /// Maps local YAML field type to the API `type` string.
    pub fn to_api_type(&self) -> &'static str {
        match self {
            FieldType::Text
            | FieldType::Richtext
            | FieldType::Date
            | FieldType::Number
            | FieldType::Boolean => "SCALAR",
            FieldType::Media => "MEDIA",
            FieldType::Entry => "ENTRY",
            FieldType::Value => "VALUE",
        }
    }

    /// Returns the scalar sub-type string for the config object, if applicable.
    pub fn scalar_subtype(&self) -> Option<&'static str> {
        match self {
            FieldType::Text => Some("text"),
            FieldType::Richtext => Some("richtext"),
            FieldType::Date => Some("date"),
            FieldType::Number => Some("number"),
            FieldType::Boolean => Some("boolean"),
            _ => None,
        }
    }

    /// Whether this field type supports a `models` reference list.
    pub fn supports_models(&self) -> bool {
        matches!(self, FieldType::Entry | FieldType::Value)
    }
}

/// A single field within a model, as represented in local YAML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalField {
    pub alias: String,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub multi: bool,
    /// For entry/value types: list of model aliases this field may reference.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

/// A model definition as represented in local YAML files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub alias: String,
    pub name: String,
    /// "entry" or "module"
    pub usage: String,
    /// List of model aliases this model inherits from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inherits: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<LocalField>,
}

/// Load all `.yml` and `.yaml` files from `dir` as `LocalModel` values.
pub fn load_models(dir: &Path) -> Result<Vec<LocalModel>> {
    if !dir.exists() {
        anyhow::bail!(
            "models directory '{}' does not exist — run `pokko init` or `pokko pull` first",
            dir.display()
        );
    }

    let mut models = Vec::new();

    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read models directory: {}", dir.display()))?;

    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && matches!(
                    p.extension().and_then(|s| s.to_str()),
                    Some("yml") | Some("yaml")
                )
        })
        .collect();

    // Sort for deterministic ordering
    paths.sort();

    for path in &paths {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;

        let model: LocalModel = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;

        models.push(model);
    }

    Ok(models)
}

/// Serialize a `LocalModel` to YAML string.
pub fn model_to_yaml(model: &LocalModel) -> Result<String> {
    serde_yaml::to_string(model).context("failed to serialize model to YAML")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_type_api_mapping() {
        assert_eq!(FieldType::Text.to_api_type(), "SCALAR");
        assert_eq!(FieldType::Richtext.to_api_type(), "SCALAR");
        assert_eq!(FieldType::Media.to_api_type(), "MEDIA");
        assert_eq!(FieldType::Entry.to_api_type(), "ENTRY");
        assert_eq!(FieldType::Value.to_api_type(), "VALUE");
    }

    #[test]
    fn test_scalar_subtype() {
        assert_eq!(FieldType::Text.scalar_subtype(), Some("text"));
        assert_eq!(FieldType::Media.scalar_subtype(), None);
        assert_eq!(FieldType::Entry.scalar_subtype(), None);
    }

    #[test]
    fn test_supports_models() {
        assert!(FieldType::Entry.supports_models());
        assert!(FieldType::Value.supports_models());
        assert!(!FieldType::Text.supports_models());
        assert!(!FieldType::Media.supports_models());
    }
}
