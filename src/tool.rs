use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    event::Event,
    extension::{
        ContextHookExecution, EventDraft, ExtensionManager, LoadedExtension,
        ToolSuggestionHookExecution,
    },
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    pub trigger: Event,
    pub current_event: Event,
    pub correlation_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolExecution {
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub events: Vec<EventDraft>,
    #[serde(skip)]
    pub host_events: Vec<EventDraft>,
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
}

impl ToolRuntime {
    pub fn new(store: SharedEventStore, extensions: Arc<ExtensionManager>) -> Result<Self> {
        let runtime = Self { store, extensions };
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

    pub fn context_hooks(&self, trigger: &Event) -> Result<Vec<ContextHookExecution>> {
        self.extensions.run_context_hooks(trigger)
    }

    pub fn tool_suggestion_hooks(
        &self,
        trigger: &Event,
    ) -> Result<Vec<ToolSuggestionHookExecution>> {
        self.extensions.run_tool_suggestion_hooks(trigger)
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
            ToolProvider::Builtin => self.execute_builtin(catalog, call, context),
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
            .unwrap_or_else(|| context.trigger.id.to_string());
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
            reaction_id: uuid("reaction_id")?,
            trigger_event_id: uuid("trigger_event_id")?,
            correlation_id: uuid("correlation_id")?,
            batch_id: string("batch_id"),
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
        let query = required_string(arguments, "query")?;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 10) as usize;
        let query_terms = search_terms(&query);
        if query_terms.is_empty() {
            bail!("tool search query must contain searchable text");
        }
        let mut matches = catalog
            .definitions()
            .iter()
            .filter(|definition| definition.name != "habibi.tools.search")
            .cloned()
            .filter_map(|definition| {
                let score = tool_search_score(&definition, &query_terms);
                (score > 0).then_some((score, definition))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        let tools = matches
            .into_iter()
            .take(limit)
            .map(|(_, definition)| {
                json!({
                    "tool": definition.name,
                    "description": definition.description,
                    "schema": definition.input_schema,
                })
            })
            .collect::<Vec<_>>();
        Ok(ToolExecution {
            result: json!({ "tools": tools }),
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

fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn tool_search_score(definition: &ToolDefinition, terms: &[String]) -> u64 {
    let name = definition.name.to_ascii_lowercase();
    let description = definition.description.to_ascii_lowercase();
    let schema = definition.input_schema.to_string().to_ascii_lowercase();
    terms
        .iter()
        .map(|term| {
            u64::from(name == *term) * 100
                + u64::from(name.contains(term)) * 20
                + u64::from(description.contains(term)) * 5
                + u64::from(schema.contains(term))
        })
        .sum()
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
                    "query":{"type":"string","description":"Describe the capability or operation needed."},
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
                    "from_event_id":{"type":"string","description":"Defaults to the reaction trigger event."},
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
                "name_prefix":{"type":"string"},"reaction_id":{"type":"string"},
                "trigger_event_id":{"type":"string"},"correlation_id":{"type":"string"},
                "batch_id":{"type":"string"},"action_id":{"type":"string"},"tool_call_id":{"type":"string"},
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
    fn tool_search_returns_only_model_facing_definition_fields() {
        let directory = tempfile::tempdir().unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let runtime = ToolRuntime::new(store, extensions).unwrap();
        let catalog = runtime.catalog().unwrap();
        let execution = runtime
            .tools_search(&catalog, &json!({ "query": "query events", "limit": 2 }))
            .unwrap();
        let first = execution.result["tools"]
            .as_array()
            .unwrap()
            .first()
            .unwrap();
        assert!(first.get("tool").is_some());
        assert!(first.get("description").is_some());
        assert!(first.get("schema").is_some());
        assert_eq!(first.as_object().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn pinned_catalog_executes_the_original_extension_generation() {
        let directory = tempfile::tempdir().unwrap();
        let extension_directory = directory.path().join("example");
        write_tool_extension(&extension_directory, "1.0.0", "old");
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let runtime = ToolRuntime::new(store, extensions.clone()).unwrap();
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
                },
                &ToolContext {
                    trigger: trigger.clone(),
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
