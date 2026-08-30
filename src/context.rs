use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{event::Event, store::SharedEventStore};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextItem {
    Event {
        event_id: Uuid,
    },
    Message {
        role: String,
        content: String,
        source_event_id: Uuid,
    },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextContribution {
    #[serde(default)]
    pub items: Vec<ContextItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompiledContext {
    pub input: Vec<Value>,
    pub source_event_count: usize,
    pub rendered_bytes: usize,
    pub estimated_tokens: usize,
    pub duplicate_items_omitted: usize,
}

pub fn compile_context_items(
    store: &SharedEventStore,
    items: &[ContextItem],
) -> Result<CompiledContext> {
    if items.len() > 500 {
        bail!("context hook returned more than 500 items");
    }
    let mut input = Vec::new();
    let mut seen = HashMap::new();
    let mut source_event_count = 0;
    let mut duplicate_items_omitted = 0;
    for item in items {
        let (deduplication_key, signature, value) = match item {
            ContextItem::Event { event_id } => {
                let key = format!("event:{event_id}");
                let stored = store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
                    .get_event(Some(&event_id.to_string()), None)?
                    .with_context(|| format!("context event '{event_id}' does not exist"))?;
                source_event_count += 1;
                let text = serde_json::to_string(&json!({
                    "context_event": stored,
                }))?;
                (key, event_id.to_string(), user_input(text))
            }
            ContextItem::Message {
                role,
                content,
                source_event_id,
            } => {
                if !matches!(role.as_str(), "user" | "assistant") {
                    bail!("context message role must be 'user' or 'assistant'");
                }
                store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
                    .get_event(Some(&source_event_id.to_string()), None)?
                    .with_context(|| {
                        format!("context message source event '{source_event_id}' does not exist")
                    })?;
                source_event_count += 1;
                (
                    format!("message:{role}:{source_event_id}"),
                    content.clone(),
                    message_input(role, content, input.len()),
                )
            }
        };
        if let Some(previous) = seen.get(&deduplication_key) {
            if previous != &signature {
                bail!("context hook returned conflicting projections for '{deduplication_key}'");
            }
            duplicate_items_omitted += 1;
        } else {
            seen.insert(deduplication_key, signature);
            input.push(value);
        }
    }
    let rendered_bytes = serde_json::to_vec(&input)?.len();
    if rendered_bytes > 2 * 1024 * 1024 {
        bail!("context hook rendered more than 2 MiB");
    }
    Ok(CompiledContext {
        input,
        source_event_count,
        rendered_bytes,
        estimated_tokens: rendered_bytes.div_ceil(4),
        duplicate_items_omitted,
    })
}

pub fn current_event_input(store: &SharedEventStore, event: &Event) -> Result<Value> {
    let mut related_results = Vec::new();
    if event.event_type == "actions.completed"
        && let Some(result_ids) = event
            .payload
            .get("batched_result_event_ids")
            .and_then(Value::as_array)
    {
        let locked = store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        for result_id in result_ids {
            let Some(result_id) = result_id.as_str() else {
                continue;
            };
            if let Some(result) = locked.get_event(Some(result_id), None)? {
                related_results.push(json!({
                    "relationship": "result",
                    "event_id": result.event.id,
                    "event_type": result.event.event_type,
                    "event": result,
                }));
            }
        }
    }
    Ok(user_input(serde_json::to_string(&json!({
        "current_event": event,
        "relationships": related_results,
    }))?))
}

fn user_input(text: String) -> Value {
    json!({
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    })
}

fn message_input(role: &str, content: &str, index: usize) -> Value {
    if role == "assistant" {
        json!({
            "type": "message",
            "id": format!("msg_habibi_context_{index}"),
            "role": "assistant",
            "status": "completed",
            "content": [{
                "type": "output_text",
                "text": content,
                "annotations": []
            }]
        })
    } else {
        user_input(content.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event::Event, store::EventStore};

    #[test]
    fn compiles_and_deduplicates_context_items() {
        let store = EventStore::open(":memory:").unwrap().shared();
        let event = Event::new(
            "chat.message.created",
            "test",
            Uuid::now_v7(),
            None,
            json!({}),
        );
        store.lock().unwrap().append(&event).unwrap();
        let items = vec![
            ContextItem::Message {
                role: "user".into(),
                content: "hello".into(),
                source_event_id: event.id,
            },
            ContextItem::Message {
                role: "user".into(),
                content: "hello".into(),
                source_event_id: event.id,
            },
        ];
        let compiled = compile_context_items(&store, &items).unwrap();
        assert_eq!(compiled.input.len(), 1);
        assert_eq!(compiled.duplicate_items_omitted, 1);
    }
}
