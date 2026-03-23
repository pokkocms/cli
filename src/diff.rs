use crate::api::{RemoteField, RemoteModel};
use crate::schema::{LocalField, LocalModel};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Change types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldPropChange {
    Name(String, String),
    Type(String, String),
    Required(bool, bool),
    Multi(bool, bool),
    /// (old aliases/IDs, new aliases)
    Models(Vec<String>, Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldChange {
    CreateField {
        field: LocalField,
    },
    UpdateField {
        id: String,
        alias: String,
        changes: Vec<FieldPropChange>,
    },
    DeleteField {
        id: String,
        alias: String,
        name: String,
    },
    /// Reorder fields — list of (alias, new_index) pairs.
    ReorderFields {
        order: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Change {
    CreateModel {
        model: LocalModel,
    },
    UpdateModel {
        id: String,
        alias: String,
        name_change: Option<(String, String)>,
        usage_change: Option<(String, String)>,
        inherits_change: Option<(Vec<String>, Vec<String>)>,
        field_changes: Vec<FieldChange>,
    },
    DeleteModel {
        id: String,
        alias: String,
        name: String,
    },
}

impl Change {
    pub fn is_destructive(&self) -> bool {
        match self {
            Change::DeleteModel { .. } => true,
            Change::UpdateModel { field_changes, .. } => {
                field_changes.iter().any(|fc| matches!(fc, FieldChange::DeleteField { .. }))
            }
            Change::CreateModel { .. } => false,
        }
    }
}

pub fn has_destructive_changes(changes: &[Change]) -> bool {
    changes.iter().any(|c| c.is_destructive())
}

// ---------------------------------------------------------------------------
// Diff logic
// ---------------------------------------------------------------------------

/// Convert a remote field's API type + config back to a local `FieldType` for
/// display/comparison purposes.
fn remote_field_local_type(remote: &RemoteField) -> String {
    match remote.field_type.as_str() {
        "MEDIA" => "media".to_string(),
        "ENTRY" => "entry".to_string(),
        "VALUE" => "value".to_string(),
        "SCALAR" => remote
            .scalar_subtype()
            .unwrap_or("text")
            .to_string(),
        other => other.to_lowercase(),
    }
}

/// Diff local fields against remote fields for a single model.
fn diff_fields(
    local_fields: &[LocalField],
    remote_fields: &[RemoteField],
) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    let mut order_changed = false;

    // Fields that exist locally
    for (local_idx, local_field) in local_fields.iter().enumerate() {
        if let Some((remote_idx, remote_field)) = remote_fields
            .iter()
            .enumerate()
            .find(|(_, rf)| rf.alias == local_field.alias)
        {
            // Field exists in both — check for property changes
            let mut prop_changes = Vec::new();

            let local_type_str = local_field.field_type.to_string();
            let remote_type_str = remote_field_local_type(remote_field);
            if local_type_str != remote_type_str {
                prop_changes.push(FieldPropChange::Type(
                    remote_type_str.clone(),
                    local_type_str.clone(),
                ));
            }

            if local_field.name != remote_field.name {
                prop_changes.push(FieldPropChange::Name(
                    remote_field.name.clone(),
                    local_field.name.clone(),
                ));
            }

            if local_field.required != remote_field.required {
                prop_changes.push(FieldPropChange::Required(
                    remote_field.required,
                    local_field.required,
                ));
            }

            if local_field.multi != remote_field.multi {
                prop_changes.push(FieldPropChange::Multi(
                    remote_field.multi,
                    local_field.multi,
                ));
            }

            // Compare models reference lists (only meaningful for entry/value).
            // Remote stores UUIDs; local stores aliases. We record both sides and
            // let apply resolve the mapping — the display layer shows old/new as-is.
            if local_field.field_type.supports_models() {
                let local_models = local_field.models.clone();
                let remote_models = remote_field.referenced_model_ids();
                if local_models != remote_models {
                    prop_changes.push(FieldPropChange::Models(
                        remote_models,
                        local_models,
                    ));
                }
            }

            if local_idx != remote_idx {
                order_changed = true;
            }

            if !prop_changes.is_empty() {
                changes.push(FieldChange::UpdateField {
                    id: remote_field.id.clone(),
                    alias: local_field.alias.clone(),
                    changes: prop_changes,
                });
            }
        } else {
            // Field only in local — create
            changes.push(FieldChange::CreateField {
                field: local_field.clone(),
            });
        }
    }

    // Fields that exist only remotely — delete
    for remote_field in remote_fields {
        if !local_fields.iter().any(|lf| lf.alias == remote_field.alias) {
            changes.push(FieldChange::DeleteField {
                id: remote_field.id.clone(),
                alias: remote_field.alias.clone(),
                name: remote_field.name.clone(),
            });
        }
    }

    // If order changed and there are no deletes, emit a ReorderFields change
    if order_changed {
        let order: Vec<String> = local_fields.iter().map(|f| f.alias.clone()).collect();
        changes.push(FieldChange::ReorderFields { order });
    }

    changes
}

/// Diff local models against remote models.
pub fn diff(local: &[LocalModel], remote: &[RemoteModel]) -> Vec<Change> {
    let mut changes = Vec::new();

    // Models that exist locally
    for local_model in local {
        if let Some(remote_model) = remote.iter().find(|rm| rm.alias == local_model.alias) {
            // Model exists in both — check for changes
            let name_change = if local_model.name != remote_model.name {
                Some((remote_model.name.clone(), local_model.name.clone()))
            } else {
                None
            };

            let local_usage = local_model.usage.to_uppercase();
            let remote_usage = remote_model.usage.to_uppercase();
            let usage_change = if local_usage != remote_usage {
                Some((remote_usage, local_usage))
            } else {
                None
            };

            // Remote stores inherits as IDs while local stores aliases. We record both
            // sides and let apply resolve the mapping at execution time.
            let inherits_change = if local_model.inherits != remote_model.inherits {
                Some((remote_model.inherits.clone(), local_model.inherits.clone()))
            } else {
                None
            };

            let field_changes = diff_fields(&local_model.fields, &remote_model.fields);

            if name_change.is_some()
                || usage_change.is_some()
                || inherits_change.is_some()
                || !field_changes.is_empty()
            {
                changes.push(Change::UpdateModel {
                    id: remote_model.id.clone(),
                    alias: local_model.alias.clone(),
                    name_change,
                    usage_change,
                    inherits_change,
                    field_changes,
                });
            }
        } else {
            // Model only in local — create
            changes.push(Change::CreateModel {
                model: local_model.clone(),
            });
        }
    }

    // Models that exist only remotely — delete
    for remote_model in remote {
        if !local.iter().any(|lm| lm.alias == remote_model.alias) {
            changes.push(Change::DeleteModel {
                id: remote_model.id.clone(),
                alias: remote_model.alias.clone(),
                name: remote_model.name.clone(),
            });
        }
    }

    changes
}

// ---------------------------------------------------------------------------
// Change summary counts
// ---------------------------------------------------------------------------

pub struct ChangeSummary {
    pub to_add: usize,
    pub to_change: usize,
    pub to_destroy: usize,
}

pub fn summarise(changes: &[Change]) -> ChangeSummary {
    let mut to_add = 0;
    let mut to_change = 0;
    let mut to_destroy = 0;

    for change in changes {
        match change {
            Change::CreateModel { .. } => to_add += 1,
            Change::UpdateModel { .. } => to_change += 1,
            Change::DeleteModel { .. } => to_destroy += 1,
        }
    }

    ChangeSummary {
        to_add,
        to_change,
        to_destroy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::FieldType;

    fn make_local_model(alias: &str, name: &str) -> LocalModel {
        LocalModel {
            alias: alias.to_string(),
            name: name.to_string(),
            usage: "entry".to_string(),
            inherits: vec![],
            fields: vec![],
        }
    }

    fn make_remote_model(id: &str, alias: &str, name: &str) -> RemoteModel {
        RemoteModel {
            id: id.to_string(),
            alias: alias.to_string(),
            name: name.to_string(),
            usage: "ENTRY".to_string(),
            inherits: vec![],
            fields: vec![],
        }
    }

    #[test]
    fn test_diff_create() {
        let local = vec![make_local_model("BlogPost", "Blog Post")];
        let remote = vec![];
        let changes = diff(&local, &remote);
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], Change::CreateModel { .. }));
    }

    #[test]
    fn test_diff_delete() {
        let local = vec![];
        let remote = vec![make_remote_model("id1", "BlogPost", "Blog Post")];
        let changes = diff(&local, &remote);
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], Change::DeleteModel { .. }));
    }

    #[test]
    fn test_diff_no_change() {
        let local = vec![make_local_model("BlogPost", "Blog Post")];
        let remote = vec![make_remote_model("id1", "BlogPost", "Blog Post")];
        let changes = diff(&local, &remote);
        assert_eq!(changes.len(), 0);
    }

    #[test]
    fn test_diff_name_change() {
        let local = vec![make_local_model("BlogPost", "Blog Post Updated")];
        let remote = vec![make_remote_model("id1", "BlogPost", "Blog Post")];
        let changes = diff(&local, &remote);
        assert_eq!(changes.len(), 1);
        if let Change::UpdateModel { name_change, .. } = &changes[0] {
            assert!(name_change.is_some());
        } else {
            panic!("expected UpdateModel");
        }
    }

    #[test]
    fn test_has_destructive_changes() {
        let changes = vec![Change::DeleteModel {
            id: "id1".to_string(),
            alias: "OldModel".to_string(),
            name: "Old Model".to_string(),
        }];
        assert!(has_destructive_changes(&changes));

        let no_destructive: Vec<Change> = vec![];
        assert!(!has_destructive_changes(&no_destructive));
    }

    #[test]
    fn test_diff_field_create() {
        let mut local = make_local_model("BlogPost", "Blog Post");
        local.fields.push(LocalField {
            alias: "title".to_string(),
            name: "Title".to_string(),
            field_type: FieldType::Text,
            required: true,
            multi: false,
            models: vec![],
        });

        let remote = vec![make_remote_model("id1", "BlogPost", "Blog Post")];
        let local = vec![local];
        let changes = diff(&local, &remote);
        assert_eq!(changes.len(), 1);
        if let Change::UpdateModel { field_changes, .. } = &changes[0] {
            assert_eq!(field_changes.len(), 1);
            assert!(matches!(field_changes[0], FieldChange::CreateField { .. }));
        } else {
            panic!("expected UpdateModel");
        }
    }
}
