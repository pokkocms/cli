use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Remote data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteField {
    pub id: String,
    pub alias: String,
    pub name: String,
    /// API-level type: "SCALAR", "MEDIA", "ENTRY", "VALUE"
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    pub multi: bool,
    pub index: i64,
    /// Raw config JSON from the API.
    #[serde(default)]
    pub config: Value,
}

impl RemoteField {
    /// Extract the scalar sub-type from config (e.g. "text", "richtext").
    pub fn scalar_subtype(&self) -> Option<&str> {
        self.config.get("type").and_then(|v| v.as_str())
    }

    /// Extract the list of model UUIDs from ENTRY/VALUE config.
    pub fn referenced_model_ids(&self) -> Vec<String> {
        self.config
            .get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModel {
    pub id: String,
    pub alias: String,
    pub name: String,
    /// API-level usage: "ENTRY" or "MODULE"
    pub usage: String,
    /// List of model UUIDs this model inherits from.
    #[serde(default)]
    pub inherits: Vec<String>,
    pub fields: Vec<RemoteField>,
}

// ---------------------------------------------------------------------------
// Mutation input types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateModelInput {
    pub project_id: String,
    pub environment_id: String,
    pub alias: String,
    pub name: String,
    pub usage: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inherits: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherits: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFieldInput {
    pub model_id: String,
    pub project_id: String,
    pub environment_id: String,
    pub alias: String,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub config: Value,
    pub required: bool,
    pub multi: bool,
    pub index: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFieldPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
}

// ---------------------------------------------------------------------------
// Project / environment listing types (used by `init`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteAccount {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteEnvironment {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteProject {
    pub id: String,
    pub name: String,
    pub key: String,
    pub account: RemoteAccount,
    /// Populated by `list_projects_with_environments`
    #[serde(default)]
    pub environments: Vec<RemoteEnvironment>,
}

// ---------------------------------------------------------------------------
// GraphQL helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

// ---------------------------------------------------------------------------
// Shared mutation strings
// ---------------------------------------------------------------------------

const UPDATE_MODEL_MUTATION: &str = r#"
    mutation UpdateModelById($id: UUID!, $patch: ModelPatch!) {
      updateModelById(input: { id: $id, patch: $patch }) {
        model { id alias }
      }
    }
"#;

const UPDATE_FIELD_MUTATION: &str = r#"
    mutation UpdateModelFieldById($id: UUID!, $patch: ModelFieldPatch!) {
      updateModelFieldById(input: { id: $id, patch: $patch }) {
        modelField { id alias }
      }
    }
"#;

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct PokkoClient {
    api_url: String,
    token: String,
    http: reqwest::Client,
}

impl PokkoClient {
    pub fn new(api_url: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            api_url: api_url.into(),
            token: token.into(),
            http,
        })
    }

    async fn graphql<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<T> {
        let body = json!({ "query": query, "variables": variables });

        let resp = self
            .http
            .post(&self.api_url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("failed to send request to Pokko API")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("API returned HTTP {}: {}", status, text);
        }

        let gql_resp: GraphQLResponse<T> = resp
            .json()
            .await
            .context("failed to deserialize API response")?;

        if let Some(errors) = gql_resp.errors {
            let msgs: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            bail!("GraphQL errors: {}", msgs.join("; "));
        }

        gql_resp
            .data
            .context("API returned no data and no errors — unexpected response")
    }

    // -----------------------------------------------------------------------
    // Query
    // -----------------------------------------------------------------------

    pub async fn list_models(&self, project: &str, environment: &str) -> Result<Vec<RemoteModel>> {
        const QUERY: &str = r#"
            query ListAllModels($project: UUID!, $environment: UUID!) {
              models(
                condition: { projectId: $project, environmentId: $environment }
                filter: { deletedAt: { isNull: true } }
                orderBy: ALIAS_ASC
              ) {
                nodes {
                  id
                  alias
                  name
                  usage
                  inherits
                  fields(
                    filter: { deletedAt: { isNull: true } }
                    orderBy: INDEX_ASC
                  ) {
                    nodes {
                      id
                      alias
                      name
                      type
                      required
                      multi
                      index
                      config
                    }
                  }
                }
              }
            }
        "#;

        #[derive(Deserialize)]
        struct FieldNode {
            id: String,
            alias: String,
            name: String,
            #[serde(rename = "type")]
            field_type: String,
            required: bool,
            multi: bool,
            index: i64,
            #[serde(default)]
            config: Value,
        }

        #[derive(Deserialize)]
        struct FieldNodes {
            nodes: Vec<FieldNode>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ModelNode {
            id: String,
            alias: String,
            name: String,
            usage: String,
            #[serde(default)]
            inherits: Vec<String>,
            fields: FieldNodes,
        }

        #[derive(Deserialize)]
        struct ModelNodes {
            nodes: Vec<ModelNode>,
        }

        #[derive(Deserialize)]
        struct Data {
            models: ModelNodes,
        }

        let data: Data = self
            .graphql(
                QUERY,
                json!({ "project": project, "environment": environment }),
            )
            .await?;

        let models = data
            .models
            .nodes
            .into_iter()
            .map(|m| RemoteModel {
                id: m.id,
                alias: m.alias,
                name: m.name,
                usage: m.usage,
                inherits: m.inherits,
                fields: m
                    .fields
                    .nodes
                    .into_iter()
                    .map(|f| RemoteField {
                        id: f.id,
                        alias: f.alias,
                        name: f.name,
                        field_type: f.field_type,
                        required: f.required,
                        multi: f.multi,
                        index: f.index,
                        config: f.config,
                    })
                    .collect(),
            })
            .collect();

        Ok(models)
    }

    // -----------------------------------------------------------------------
    // Project / environment queries (used by `init`)
    // -----------------------------------------------------------------------

    /// List all projects the authenticated user has access to, each with their
    /// available environments.
    pub async fn list_projects(&self) -> Result<Vec<RemoteProject>> {
        const QUERY: &str = r#"
            query ListProjectsWithEnvironments {
              projects(first: 50, orderBy: NAME_ASC, filter: { deletedAt: { isNull: true } }) {
                nodes {
                  id
                  name
                  key
                  account { id name }
                  projectEnvironments(
                    first: 25
                    filter: { deletedAt: { isNull: true } }
                    orderBy: NAME_ASC
                  ) {
                    nodes { id name key }
                  }
                }
              }
            }
        "#;

        #[derive(Deserialize)]
        struct EnvNode {
            id: String,
            name: String,
            key: String,
        }
        #[derive(Deserialize)]
        struct EnvNodes {
            nodes: Vec<EnvNode>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ProjNode {
            id: String,
            name: String,
            key: String,
            account: RemoteAccount,
            project_environments: EnvNodes,
        }
        #[derive(Deserialize)]
        struct ProjNodes {
            nodes: Vec<ProjNode>,
        }
        #[derive(Deserialize)]
        struct Data {
            projects: ProjNodes,
        }

        let data: Data = self.graphql(QUERY, json!({})).await?;

        Ok(data
            .projects
            .nodes
            .into_iter()
            .map(|p| RemoteProject {
                id: p.id,
                name: p.name,
                key: p.key,
                account: p.account,
                environments: p
                    .project_environments
                    .nodes
                    .into_iter()
                    .map(|e| RemoteEnvironment {
                        id: e.id,
                        name: e.name,
                        key: e.key,
                    })
                    .collect(),
            })
            .collect())
    }

    // -----------------------------------------------------------------------
    // Model mutations
    // -----------------------------------------------------------------------

    pub async fn create_model(&self, input: &CreateModelInput) -> Result<String> {
        const MUTATION: &str = r#"
            mutation CreateModel($input: ModelInput!) {
              createModel(input: { model: $input }) {
                model { id alias }
              }
            }
        "#;

        #[derive(Deserialize)]
        struct ModelId {
            id: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateModel {
            model: ModelId,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            create_model: CreateModel,
        }

        let input_val = serde_json::to_value(input).context("failed to serialize model input")?;
        let data: Data = self
            .graphql(MUTATION, json!({ "input": input_val }))
            .await?;

        Ok(data.create_model.model.id)
    }

    pub async fn update_model(&self, id: &str, patch: &UpdateModelPatch) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "updateModelById")]
            _update: Value,
        }

        let patch_val = serde_json::to_value(patch).context("failed to serialize model patch")?;
        let _: Data = self
            .graphql(UPDATE_MODEL_MUTATION, json!({ "id": id, "patch": patch_val }))
            .await?;

        Ok(())
    }

    pub async fn soft_delete_model(&self, id: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "updateModelById")]
            _update: Value,
        }

        let patch = json!({ "deletedAt": Utc::now().to_rfc3339() });
        let _: Data = self
            .graphql(UPDATE_MODEL_MUTATION, json!({ "id": id, "patch": patch }))
            .await?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Field mutations
    // -----------------------------------------------------------------------

    pub async fn create_field(&self, input: &CreateFieldInput) -> Result<String> {
        const MUTATION: &str = r#"
            mutation CreateModelField($input: ModelFieldInput!) {
              createModelField(input: { modelField: $input }) {
                modelField { id alias }
              }
            }
        "#;

        #[derive(Deserialize)]
        struct FieldId {
            id: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CreateModelField {
            model_field: FieldId,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            create_model_field: CreateModelField,
        }

        let input_val = serde_json::to_value(input).context("failed to serialize field input")?;
        let data: Data = self
            .graphql(MUTATION, json!({ "input": input_val }))
            .await?;

        Ok(data.create_model_field.model_field.id)
    }

    pub async fn update_field(&self, id: &str, patch: &UpdateFieldPatch) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "updateModelFieldById")]
            _update: Value,
        }

        let patch_val = serde_json::to_value(patch).context("failed to serialize field patch")?;
        let _: Data = self
            .graphql(UPDATE_FIELD_MUTATION, json!({ "id": id, "patch": patch_val }))
            .await?;

        Ok(())
    }

    pub async fn soft_delete_field(&self, id: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "updateModelFieldById")]
            _update: Value,
        }

        let patch = json!({ "deletedAt": Utc::now().to_rfc3339() });
        let _: Data = self
            .graphql(UPDATE_FIELD_MUTATION, json!({ "id": id, "patch": patch }))
            .await?;

        Ok(())
    }

    /// Reorder fields by updating each field's index.
    pub async fn reorder_fields(&self, ordered_ids: &[(String, i64)]) -> Result<()> {
        for (id, index) in ordered_ids {
            let patch = UpdateFieldPatch {
                name: None,
                field_type: None,
                config: None,
                required: None,
                multi: None,
                index: Some(*index),
            };
            self.update_field(id, &patch).await?;
        }
        Ok(())
    }
}
