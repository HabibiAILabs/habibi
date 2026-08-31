use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_SEMANTIC_SEARCH_QUERY_BYTES: usize = 4_096;

use crate::{
    embedding::{
        Embedder, FINAL_TOOL_LIMIT, MIN_TOOL_SIMILARITY, SEMANTIC_TOOL_LIMIT, ToolEmbeddingIndex,
        event_tool_query,
    },
    event::Event,
    extension::{ContextHookExecution, EventDraft, ExtensionManager, LoadedExtension},
    store::{SharedEventStore, StoreEventQuery, StoreLogQuery},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    pub current_event: Event,
    pub correlation_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct HostEffect {
    pub source: &'static str,
    pub event: EventDraft,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolExecution {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub events: Vec<EventDraft>,
    #[serde(skip)]
    pub host_events: Vec<HostEffect>,
    #[serde(skip)]
    pub failure: Option<String>,
}

#[derive(Clone)]
enum ToolProvider {
    Builtin,
    Extension(Arc<LoadedExtension>),
}

#[derive(Clone)]
pub struct ToolCatalog {
    pub generation: String,
    definitions: Vec<ToolDefinition>,
    providers: BTreeMap<String, ToolProvider>,
}

impl ToolCatalog {
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.definitions
    }

    pub fn definition(&self, name: &str) -> Option<ToolDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.name == name)
            .cloned()
    }
}

#[derive(Clone)]
pub struct ToolRuntime {
    store: SharedEventStore,
    extensions: Arc<ExtensionManager>,
    embeddings: Arc<ToolEmbeddingIndex>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectedTool {
    pub tool: String,
    pub score: Option<f32>,
    pub rank: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ToolSelection {
    pub records: Vec<SelectedTool>,
    pub query_sha256: String,
}

impl ToolRuntime {
    pub fn new(
        store: SharedEventStore,
        extensions: Arc<ExtensionManager>,
        embedder: Arc<dyn Embedder>,
    ) -> Result<Self> {
        let embeddings = Arc::new(ToolEmbeddingIndex::new(embedder, store.clone()));
        let runtime = Self {
            store,
            extensions,
            embeddings,
        };
        runtime.catalog()?;
        Ok(runtime)
    }

    pub fn catalog(&self) -> Result<Arc<ToolCatalog>> {
        let mut providers = BTreeMap::new();
        let mut definitions = builtin_definitions();
        for definition in &definitions {
            providers.insert(definition.name.clone(), ToolProvider::Builtin);
        }
        let mut fingerprints = definitions
            .iter()
            .map(|definition| json!({ "definition": definition, "provider": "core" }))
            .collect::<Vec<_>>();
        for (definition, extension) in self.extensions.tool_catalog_entries() {
            if providers
                .insert(
                    definition.name.clone(),
                    ToolProvider::Extension(extension.clone()),
                )
                .is_some()
            {
                bail!("duplicate tool name '{}'", definition.name);
            }
            fingerprints.push(json!({
                "definition": &definition,
                "extension": extension.manifest.id,
                "version": extension.manifest.version,
                "extension_generation": extension.generation,
            }));
            definitions.push(definition);
        }
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        fingerprints.sort_by_key(Value::to_string);
        let mut provider_names = std::collections::HashSet::new();
        for definition in &definitions {
            jsonschema::validator_for(&definition.input_schema).with_context(|| {
                format!("tool '{}' has an invalid input schema", definition.name)
            })?;
            let provider_name = provider_tool_name(&definition.name);
            if !provider_names.insert(provider_name.clone()) {
                bail!("tool names collide after provider normalization: '{provider_name}'");
            }
        }
        let generation = format!("{:x}", Sha256::digest(serde_json::to_vec(&fingerprints)?));
        Ok(Arc::new(ToolCatalog {
            generation,
            definitions,
            providers,
        }))
    }

    pub async fn initialize_catalog(&self) -> Result<()> {
        let catalog = self.catalog()?;
        let embeddings = self.embeddings.clone();
        tokio::task::spawn_blocking(move || {
            embeddings.ensure_catalog(&catalog.generation, catalog.definitions())
        })
        .await
        .context("tool embedding index initialization task failed")??;
        Ok(())
    }

    pub fn context_hooks(&self, trigger: &Event) -> Result<Vec<ContextHookExecution>> {
        self.extensions.run_context_hooks(trigger)
    }

    pub fn embedding_model(&self) -> &str {
        self.embeddings.model_id()
    }

    pub fn embedding_revision(&self) -> &str {
        self.embeddings.revision()
    }

    pub async fn select_tools(
        &self,
        catalog: Arc<ToolCatalog>,
        event: &Event,
        compiled_context: &[Value],
    ) -> Result<ToolSelection> {
        let query = event_tool_query(event, compiled_context);
        let query_sha256 = format!("{:x}", Sha256::digest(query.as_bytes()));
        let embeddings = self.embeddings.clone();
        let generation = catalog.generation.clone();
        let definitions = catalog.definitions.clone();
        let correlation_id = event.correlation_id;
        let discovered = (event.payload.get("tool").and_then(Value::as_str)
            == Some("habibi.tools.search"))
        .then(|| {
            event
                .payload
                .pointer("/result/tools")
                .and_then(Value::as_array)
        })
        .flatten()
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            Some(crate::embedding::SemanticToolMatch {
                tool: tool.get("tool")?.as_str()?.to_owned(),
                score: tool.get("score")?.as_f64()? as f32,
                rank: tool.get("rank")?.as_u64()? as usize,
            })
        })
        .collect::<Vec<_>>();
        let (mut semantic, used) = tokio::task::spawn_blocking(move || {
            let semantic = embeddings.search(
                &generation,
                &definitions,
                &query,
                SEMANTIC_TOOL_LIMIT,
                MIN_TOOL_SIMILARITY,
            )?;
            let used = embeddings.used_tools(correlation_id)?;
            Ok::<_, anyhow::Error>((semantic, used))
        })
        .await
        .context("semantic tool selection task failed")??;
        for candidate in discovered {
            if !semantic
                .iter()
                .any(|existing| existing.tool == candidate.tool)
            {
                semantic.push(candidate);
            }
        }
        semantic.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.tool.cmp(&right.tool))
        });
        let registered = catalog
            .definitions
            .iter()
            .map(|definition| definition.name.clone())
            .collect::<HashSet<_>>();
        let records = merge_tool_candidates(&registered, used, semantic);
        Ok(ToolSelection {
            records,
            query_sha256,
        })
    }

    pub async fn execute(
        &self,
        catalog: &ToolCatalog,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<ToolExecution> {
        let provider = catalog
            .providers
            .get(&call.name)
            .with_context(|| format!("tool '{}' is not in the pinned catalog", call.name))?;
        match provider {
            ToolProvider::Builtin => {
                let runtime = self.clone();
                let catalog = catalog.clone();
                let call = call.clone();
                let context = context.clone();
                tokio::task::spawn_blocking(move || {
                    runtime.execute_builtin(&catalog, &call, &context)
                })
                .await
                .context("built-in tool task failed")?
            }
            ToolProvider::Extension(extension) => {
                let extension = extension.clone();
                let call = call.clone();
                let context = context.clone();
                tokio::task::spawn_blocking(move || extension.execute_tool(&call, &context))
                    .await
                    .context("extension tool task failed")?
            }
        }
    }

    fn execute_builtin(
        &self,
        catalog: &ToolCatalog,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<ToolExecution> {
        match call.name.as_str() {
            "habibi.events.get" => self.events_get(&call.arguments),
            "habibi.events.query" => self.events_query(&call.arguments),
            "habibi.events.link" => self.events_link(&call.arguments, context),
            "habibi.events.related" => self.events_related(&call.arguments),
            "habibi.logs.get" => self.logs_get(&call.arguments),
            "habibi.logs.query" => self.logs_query(&call.arguments),
            "habibi.tools.search" => self.tools_search(catalog, &call.arguments),
            _ => bail!("unknown built-in tool '{}'", call.name),
        }
    }

    fn events_get(&self, arguments: &Value) -> Result<ToolExecution> {
        let event_id = arguments.get("event_id").and_then(Value::as_str);
        let sequence = arguments.get("sequence").and_then(Value::as_i64);
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        let event = if let Some(event_id) = event_id {
            store.get_event(Some(event_id), None)?
        } else if let Some(sequence) = sequence {
            store.get_event(None, Some(sequence))?
        } else {
            bail!("event_id or sequence is required");
        };
        Ok(ToolExecution {
            result: json!({ "event": event }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }

    fn events_query(&self, arguments: &Value) -> Result<ToolExecution> {
        let string = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let parse_time = |key: &str| -> Result<Option<DateTime<Utc>>> {
            string(key)
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&Utc))
                        .map_err(Into::into)
                })
                .transpose()
        };
        let query = StoreEventQuery {
            event_type: string("event_type"),
            event_type_prefix: string("event_type_prefix"),
            source: string("source"),
            event_id: string("event_id")
                .map(|value| Uuid::parse_str(&value))
                .transpose()?,
            causation_id: string("causation_id")
                .map(|value| Uuid::parse_str(&value))
                .transpose()?,
            correlation_id: string("correlation_id")
                .map(|value| Uuid::parse_str(&value))
                .transpose()?,
            before_sequence: arguments.get("before_sequence").and_then(Value::as_i64),
            after_sequence: arguments.get("after_sequence").and_then(Value::as_i64),
            occurred_after: parse_time("occurred_after")?,
            occurred_before: parse_time("occurred_before")?,
            payload_contains: string("payload_contains"),
            limit: arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 1000) as usize,
        };
        let events = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .query_events(&query)?;
        Ok(ToolExecution {
            result: json!({ "events": events }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }

    fn events_link(&self, arguments: &Value, context: &ToolContext) -> Result<ToolExecution> {
        let from_event_id = arguments
            .get("from_event_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| context.current_event.id.to_string());
        let to_event_id = required_string(arguments, "to_event_id")?;
        let relation = arguments
            .get("relation")
            .and_then(Value::as_str)
            .unwrap_or("related")
            .to_owned();
        let description = arguments
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let bidirectional = arguments
            .get("bidirectional")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let (from_event_type, to_event_type) = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
            let from = store
                .get_event(Some(&from_event_id), None)?
                .context("linked source event must exist")?;
            let to = store
                .get_event(Some(&to_event_id), None)?
                .context("linked target event must exist")?;
            (from.event.event_type, to.event.event_type)
        };
        let link_id = Uuid::now_v7().to_string();
        Ok(ToolExecution {
            result: json!({ "link_id": link_id, "linked": true }),
            events: vec![EventDraft {
                event_type: "event.link.created".into(),
                idempotency_key: None,
                payload: json!({
                    "link_id": link_id,
                    "from_event_id": from_event_id,
                    "from_event_type": from_event_type,
                    "to_event_id": to_event_id,
                    "to_event_type": to_event_type,
                    "relation": relation,
                    "description": description,
                    "bidirectional": bidirectional
                }),
            }],
            host_events: vec![],
            failure: None,
        })
    }

    fn logs_get(&self, arguments: &Value) -> Result<ToolExecution> {
        let log_id = required_string(arguments, "log_id")?;
        let log = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .get_log(&log_id)?;
        Ok(ToolExecution {
            result: json!({ "log": log }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }

    fn logs_query(&self, arguments: &Value) -> Result<ToolExecution> {
        let string = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let uuid = |key: &str| -> Result<Option<Uuid>> {
            string(key)
                .map(|value| Uuid::parse_str(&value).map_err(Into::into))
                .transpose()
        };
        let time = |key: &str| -> Result<Option<DateTime<Utc>>> {
            string(key)
                .map(|value| {
                    DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&Utc))
                        .map_err(Into::into)
                })
                .transpose()
        };
        let query = StoreLogQuery {
            level: string("level"),
            category: string("category"),
            name: string("name"),
            name_prefix: string("name_prefix"),
            dispatch_id: uuid("dispatch_id")?,
            event_id: uuid("event_id")?,
            correlation_id: uuid("correlation_id")?,
            action_group_id: string("action_group_id"),
            action_id: string("action_id"),
            tool_call_id: string("tool_call_id"),
            before_sequence: arguments.get("before_sequence").and_then(Value::as_i64),
            after_sequence: arguments.get("after_sequence").and_then(Value::as_i64),
            occurred_after: time("occurred_after")?,
            occurred_before: time("occurred_before")?,
            payload_contains: string("payload_contains"),
            limit: arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 1000) as usize,
        };
        let logs = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .query_logs(&query)?;
        Ok(ToolExecution {
            result: json!({ "logs": logs }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }

    fn tools_search(&self, catalog: &ToolCatalog, arguments: &Value) -> Result<ToolExecution> {
        let query = bounded_search_query(arguments)?;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 10) as usize;
        let matches = self.embeddings.search(
            &catalog.generation,
            catalog.definitions(),
            &query,
            limit.saturating_add(1),
            MIN_TOOL_SIMILARITY,
        )?;
        let tools = matches
            .into_iter()
            .filter(|candidate| candidate.tool != "habibi.tools.search")
            .take(limit)
            .filter_map(|candidate| {
                catalog.definition(&candidate.tool).map(|definition| {
                    json!({
                        "tool": definition.name,
                        "description": definition.description,
                        "schema": definition.input_schema,
                        "score": candidate.score,
                        "rank": candidate.rank,
                    })
                })
            })
            .collect::<Vec<_>>();
        Ok(ToolExecution {
            result: json!({
                "tools": tools,
                "embedding_model": self.embeddings.model_id(),
                "embedding_revision": self.embeddings.revision(),
                "minimum_similarity": MIN_TOOL_SIMILARITY,
            }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }

    fn events_related(&self, arguments: &Value) -> Result<ToolExecution> {
        let event_id = required_string(arguments, "event_id")?;
        let relation = arguments.get("relation").and_then(Value::as_str);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 500) as usize;
        let links = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .related_events(&event_id, relation, limit)?;
        Ok(ToolExecution {
            result: json!({ "links": links }),
            events: vec![],
            host_events: vec![],
            failure: None,
        })
    }
}

fn merge_tool_candidates(
    registered: &HashSet<String>,
    used: Vec<String>,
    semantic: Vec<crate::embedding::SemanticToolMatch>,
) -> Vec<SelectedTool> {
    let semantic_by_name = semantic
        .iter()
        .map(|candidate| (candidate.tool.as_str(), candidate))
        .collect::<std::collections::HashMap<_, _>>();
    let mut records = Vec::new();
    let mut selected = HashSet::new();
    for tool in used {
        if records.len() == FINAL_TOOL_LIMIT {
            break;
        }
        if !registered.contains(&tool) || !selected.insert(tool.clone()) {
            continue;
        }
        let semantic = semantic_by_name.get(tool.as_str()).copied();
        records.push(SelectedTool {
            tool,
            score: semantic.map(|candidate| candidate.score),
            rank: semantic.map(|candidate| candidate.rank),
            reason: if semantic.is_some() {
                "both"
            } else {
                "used_in_correlation"
            }
            .into(),
        });
    }
    for candidate in semantic {
        if records.len() == FINAL_TOOL_LIMIT {
            break;
        }
        if registered.contains(&candidate.tool) && selected.insert(candidate.tool.clone()) {
            records.push(SelectedTool {
                tool: candidate.tool,
                score: Some(candidate.score),
                rank: Some(candidate.rank),
                reason: "semantic_match".into(),
            });
        }
    }
    records
}

fn bounded_search_query(arguments: &Value) -> Result<String> {
    let query = required_string(arguments, "query")?;
    if query.len() > MAX_SEMANTIC_SEARCH_QUERY_BYTES {
        bail!("'query' must not exceed {MAX_SEMANTIC_SEARCH_QUERY_BYTES} UTF-8 bytes");
    }
    Ok(query)
}

fn required_string(arguments: &Value, key: &str) -> Result<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("'{key}' is required"))
}

pub fn provider_tool_name(name: &str) -> String {
    name.replace('.', "__")
}

pub fn domain_tool_name<'a>(
    provider_name: &str,
    definitions: &'a [ToolDefinition],
) -> Option<&'a str> {
    definitions
        .iter()
        .find(|definition| provider_tool_name(&definition.name) == provider_name)
        .map(|definition| definition.name.as_str())
}

fn builtin_definitions() -> Vec<ToolDefinition> {
    vec![
        definition(
            "habibi.tools.search",
            "Search the installed tool registry by capability. Returns matching tool names, descriptions, and schemas.",
            json!({
                "type":"object",
                "properties":{
                    "query":{"type":"string","minLength":1,"maxLength":4096,"description":"Describe the capability or operation needed. Maximum 4096 UTF-8 bytes."},
                    "limit":{"type":"integer","minimum":1,"maximum":10}
                },
                "required":["query"]
            }),
        ),
        definition(
            "habibi.events.get",
            "Get one event by event ID or sequence.",
            json!({
                "type":"object","properties":{"event_id":{"type":"string"},"sequence":{"type":"integer"}}
            }),
        ),
        definition(
            "habibi.events.query",
            "Query the durable event stream by envelope metadata, time, or payload text.",
            json!({
                "type":"object","properties":{
                    "event_type":{"type":"string"},"event_type_prefix":{"type":"string"},"source":{"type":"string"},
                    "event_id":{"type":"string"},"causation_id":{"type":"string"},"correlation_id":{"type":"string"},
                    "before_sequence":{"type":"integer"},"after_sequence":{"type":"integer"},
                    "occurred_after":{"type":"string"},"occurred_before":{"type":"string"},"payload_contains":{"type":"string"},
                    "limit":{"type":"integer","minimum":1,"maximum":1000}
                }
            }),
        ),
        definition(
            "habibi.events.link",
            "Create an agent-authored semantic link between two existing events.",
            json!({
                "type":"object","properties":{
                    "from_event_id":{"type":"string","description":"Defaults to the current event."},
                    "to_event_id":{"type":"string"},"relation":{"type":"string","enum":["related","continues","supports","contradicts","derived_from","supersedes","same_entity","same_topic"]},
                    "description":{"type":"string"},"bidirectional":{"type":"boolean"}
                },"required":["to_event_id"]
            }),
        ),
        definition(
            "habibi.events.related",
            "Traverse semantic links connected to an event.",
            json!({
                "type":"object","properties":{"event_id":{"type":"string"},"relation":{"type":"string"},"limit":{"type":"integer"}},"required":["event_id"]
            }),
        ),
        definition(
            "habibi.logs.get",
            "Get one operational log record by ID.",
            json!({"type":"object","properties":{"log_id":{"type":"string"}},"required":["log_id"]}),
        ),
        definition(
            "habibi.logs.query",
            "Search detailed operational logs for model, action, extension, HTTP, and runtime execution.",
            json!({"type":"object","properties":{
                "level":{"type":"string"},"category":{"type":"string"},"name":{"type":"string"},
                "name_prefix":{"type":"string"},"dispatch_id":{"type":"string"},
                "event_id":{"type":"string"},"correlation_id":{"type":"string"},
                "action_group_id":{"type":"string"},"action_id":{"type":"string"},"tool_call_id":{"type":"string"},
                "before_sequence":{"type":"integer"},"after_sequence":{"type":"integer"},
                "occurred_after":{"type":"string"},"occurred_before":{"type":"string"},
                "payload_contains":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":1000}
            }}),
        ),
    ]
}

fn definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extension::ExtensionManager, store::EventStore};

    fn write_tool_extension(directory: &std::path::Path, version: &str, result: &str) {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("extension.toml"),
            format!(
                "id = \"example\"\nname = \"Example\"\nversion = \"{version}\"\napi_version = 2\n[capabilities]\ntools = true\n"
            ),
        )
        .unwrap();
        std::fs::write(
            directory.join("extension.lua"),
            format!(
                "habibi.tools.register({{ name = \"example.read\", description = \"Read\", input_schema = {{ type = \"object\" }} }}, function() return {{ result = {{ value = \"{result}\" }} }} end)"
            ),
        )
        .unwrap();
    }

    #[test]
    fn semantic_and_used_union_is_deduplicated_used_first_and_bounded() {
        let registered = (0..80)
            .map(|index| format!("tool.{index:02}"))
            .collect::<HashSet<_>>();
        let used = (0..50)
            .rev()
            .map(|index| format!("tool.{index:02}"))
            .collect::<Vec<_>>();
        let semantic = (0..80)
            .map(|index| crate::embedding::SemanticToolMatch {
                tool: format!("tool.{index:02}"),
                score: 1.0 - index as f32 / 100.0,
                rank: index + 1,
            })
            .collect::<Vec<_>>();
        let selected = merge_tool_candidates(&registered, used, semantic);
        assert_eq!(selected.len(), FINAL_TOOL_LIMIT);
        assert_eq!(selected[0].tool, "tool.49");
        assert_eq!(selected[0].reason, "both");
        assert_eq!(
            selected
                .iter()
                .map(|record| &record.tool)
                .collect::<HashSet<_>>()
                .len(),
            FINAL_TOOL_LIMIT
        );
    }

    #[test]
    fn rejects_invalid_tool_schemas_before_advertisement() {
        let directory = tempfile::tempdir().unwrap();
        let extension_directory = directory.path().join("example");
        std::fs::create_dir_all(&extension_directory).unwrap();
        std::fs::write(
            extension_directory.join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 2\n[capabilities]\ntools = true\n",
        )
        .unwrap();
        std::fs::write(
            extension_directory.join("extension.lua"),
            "habibi.tools.register({ name = 'example.bad', description = 'Bad', input_schema = { type = 'object', required = 'message' } }, function() return {} end)",
        )
        .unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let error = ToolRuntime::new(
            store,
            extensions,
            Arc::new(crate::embedding::DeterministicTestEmbedder),
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("example.bad"), "{error}");
        assert!(error.contains("invalid input schema"), "{error}");
    }

    #[test]
    fn tool_search_returns_only_model_facing_definition_fields() {
        let directory = tempfile::tempdir().unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let runtime = ToolRuntime::new(
            store,
            extensions,
            Arc::new(crate::embedding::DeterministicTestEmbedder),
        )
        .unwrap();
        let catalog = runtime.catalog().unwrap();
        let execution = runtime
            .tools_search(&catalog, &json!({ "query": "query events", "limit": 10 }))
            .unwrap();
        let first = execution.result["tools"]
            .as_array()
            .unwrap()
            .first()
            .unwrap();
        assert!(first.get("tool").is_some());
        assert!(first.get("description").is_some());
        assert!(first.get("schema").is_some());
        assert!(first.get("score").is_some());
        assert!(first.get("rank").is_some());
        assert_eq!(first.as_object().unwrap().len(), 5);
    }

    #[test]
    fn semantic_tool_search_query_is_bounded_before_inference() {
        assert_eq!(
            bounded_search_query(&json!({ "query": "x".repeat(MAX_SEMANTIC_SEARCH_QUERY_BYTES) }))
                .unwrap()
                .len(),
            MAX_SEMANTIC_SEARCH_QUERY_BYTES
        );
        let error = bounded_search_query(
            &json!({ "query": "x".repeat(MAX_SEMANTIC_SEARCH_QUERY_BYTES + 1) }),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("4096 UTF-8 bytes"), "{error}");
    }

    #[tokio::test]
    async fn pinned_catalog_executes_the_original_extension_generation() {
        let directory = tempfile::tempdir().unwrap();
        let extension_directory = directory.path().join("example");
        write_tool_extension(&extension_directory, "1.0.0", "old");
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let runtime = ToolRuntime::new(
            store,
            extensions.clone(),
            Arc::new(crate::embedding::DeterministicTestEmbedder),
        )
        .unwrap();
        let catalog = runtime.catalog().unwrap();
        write_tool_extension(&extension_directory, "2.0.0", "new");
        extensions.reload("example").unwrap();
        let trigger = Event::new("test.trigger", "test", Uuid::now_v7(), None, json!({}));
        let execution = runtime
            .execute(
                &catalog,
                &ToolCall {
                    call_id: "call-1".into(),
                    name: "example.read".into(),
                    arguments: json!({}),
                    argument_error: None,
                },
                &ToolContext {
                    current_event: trigger.clone(),
                    correlation_id: trigger.correlation_id,
                },
            )
            .await
            .unwrap();
        assert_eq!(execution.result["value"], "old");
        assert_ne!(catalog.generation, runtime.catalog().unwrap().generation);
    }
}
