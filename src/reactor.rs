use std::{collections::VecDeque, sync::Arc, time::Instant};

use anyhow::{Context, Result};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    catalog::ModelCatalog,
    event::{ConversationMessage, Event, LogEntry},
    model::ModelClient,
    store::SharedEventStore,
    tool::{ToolCall, ToolContext, ToolRuntime, domain_tool_name},
};

pub struct Reactor {
    store: SharedEventStore,
    model: ModelClient,
    tools: Arc<ToolRuntime>,
    context_message_limit: usize,
}

impl Reactor {
    pub fn new(
        store: SharedEventStore,
        model: ModelClient,
        tools: Arc<ToolRuntime>,
        context_message_limit: usize,
    ) -> Self {
        Self {
            store,
            model,
            tools,
            context_message_limit,
        }
    }

    pub fn model_catalog(&self) -> Result<ModelCatalog> {
        self.model.catalog()
    }

    pub async fn refresh_model_catalog(&self) -> Result<ModelCatalog> {
        let catalog = self.model.refresh_catalog().await?;
        let reaction_id = Uuid::now_v7();
        self.log(LogEntry::new(
            "info",
            "model",
            "model.catalog.refreshed",
            reaction_id,
            None,
            reaction_id,
            json!({
                "source": catalog.source,
                "updated_at": catalog.updated_at,
                "model_count": catalog.models.len()
            }),
        ))?;
        Ok(catalog)
    }

    pub fn record_runtime_started(&self) -> Result<()> {
        let reaction_id = Uuid::now_v7();
        self.log(LogEntry::new(
            "info",
            "runtime",
            "runtime.started",
            reaction_id,
            None,
            reaction_id,
            json!({ "model": self.model.model_name() }),
        ))?;
        Ok(())
    }

    pub async fn react(
        &self,
        trigger: &Event,
        mut conversation: Vec<ConversationMessage>,
    ) -> Result<()> {
        if conversation.len() > self.context_message_limit {
            conversation.drain(..conversation.len() - self.context_message_limit);
        }
        let reaction_id = trigger.correlation_id;
        self.log(LogEntry::new(
            "info",
            "reactor",
            "reaction.started",
            reaction_id,
            Some(trigger.id),
            trigger.correlation_id,
            json!({ "trigger_event": trigger }),
        ))?;

        let definitions = self.tools.definitions();
        let mut initial_input = self.model.conversation_input(&conversation);
        initial_input.push(current_event_input(trigger)?);
        let mut queue = VecDeque::from([PendingModelEvent {
            event: trigger.clone(),
            input: initial_input,
        }]);
        let mut processed_event_ids = Vec::new();
        let mut final_event_id = trigger.id;

        while let Some(pending) = queue.pop_front() {
            let current_event = pending.event;
            final_event_id = current_event.id;
            processed_event_ids.push(current_event.id);
            self.log(LogEntry::new(
                "debug",
                "reactor",
                "event.processing.started",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "event_id": current_event.id,
                    "event_type": current_event.event_type,
                    "root_trigger_event_id": trigger.id
                }),
            ))?;

            let request = self.model.request_body(&pending.input, &definitions);
            let model_span_id = Uuid::now_v7().to_string();
            let mut started_log = LogEntry::new(
                "info",
                "model",
                "model.invocation.started",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "provider": "openai-codex", "model": self.model.model_name(),
                    "endpoint": self.model.endpoint(), "root_trigger_event_id": trigger.id,
                    "current_event_id": current_event.id, "request": request
                }),
            );
            started_log.span_id = Some(model_span_id.clone());
            let started_log_id = started_log.id;
            self.log(started_log)?;

            let invocation_started_at = Instant::now();
            let mut response = match self.model.invoke(request).await {
                Ok(response) => response,
                Err(error) => {
                    let mut failed = LogEntry::new(
                        "error",
                        "model",
                        "model.invocation.failed",
                        reaction_id,
                        Some(current_event.id),
                        trigger.correlation_id,
                        json!({
                            "error": error.to_string(), "started_log_id": started_log_id,
                            "duration_ms": invocation_started_at.elapsed().as_millis()
                        }),
                    );
                    failed.parent_span_id = Some(model_span_id);
                    self.log(failed)?;
                    return Err(error);
                }
            };
            for call in &mut response.tool_calls {
                call.name = domain_tool_name(&call.name, &definitions)
                    .with_context(|| format!("model called unknown tool '{}'", call.name))?
                    .to_owned();
            }

            let estimated_cost = response
                .usage
                .as_ref()
                .and_then(|usage| self.model.estimate_cost(usage));
            let mut completed_log = LogEntry::new(
                "info",
                "model",
                "model.invocation.completed",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "started_log_id": started_log_id, "provider": response.provider,
                    "model": response.model, "content": &response.content,
                    "tool_calls": &response.tool_calls, "output_items": &response.output_items,
                    "provider_response": &response.provider_response,
                    "usage": &response.usage,
                    "estimated_cost": &estimated_cost,
                    "duration_ms": invocation_started_at.elapsed().as_millis()
                }),
            );
            completed_log.parent_span_id = Some(model_span_id);
            let completed_log_id = completed_log.id;
            self.log(completed_log)?;

            if response.tool_calls.is_empty() {
                self.log(LogEntry::new(
                    "debug",
                    "reactor",
                    "event.processing.completed",
                    reaction_id,
                    Some(current_event.id),
                    trigger.correlation_id,
                    json!({ "event_id": current_event.id, "outcome": "no_actions" }),
                ))?;
                continue;
            }

            let batch = self
                .execute_batch(
                    trigger,
                    &current_event,
                    completed_log_id,
                    &response.tool_calls,
                )
                .await?;
            self.log(LogEntry::new(
                "debug",
                "reactor",
                "event.processing.completed",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "event_id": current_event.id,
                    "outcome": "actions_requested",
                    "next_event_id": batch.completed_event.id
                }),
            ))?;

            let mut next_input = self.model.conversation_input(&conversation);
            next_input.extend(response.output_items);
            for result in &batch.results {
                next_input.push(json!({
                    "type": "function_call_output", "call_id": result.call.call_id,
                    "output": serde_json::to_string(&result.output)?
                }));
            }
            next_input.push(current_event_input(&batch.completed_event)?);
            queue.push_back(PendingModelEvent {
                event: batch.completed_event,
                input: next_input,
            });
        }

        self.log(LogEntry::new(
            "info",
            "reactor",
            "reaction.settled",
            reaction_id,
            Some(final_event_id),
            trigger.correlation_id,
            json!({
                "reason": "event_queue_empty",
                "processed_model_event_ids": processed_event_ids
            }),
        ))?;
        Ok(())
    }

    async fn execute_batch(
        &self,
        root_trigger: &Event,
        current_event: &Event,
        model_log_id: Uuid,
        calls: &[ToolCall],
    ) -> Result<BatchExecution> {
        let batch_id = Uuid::now_v7();
        let mut batch_log = LogEntry::new(
            "info",
            "action",
            "action.batch.created",
            root_trigger.correlation_id,
            Some(current_event.id),
            root_trigger.correlation_id,
            json!({ "batch_id": batch_id, "call_count": calls.len(), "model_log_id": model_log_id }),
        );
        batch_log.batch_id = Some(batch_id.to_string());
        self.log(batch_log)?;

        let mut actions = Vec::new();
        let mut request_event_ids = Vec::new();
        for (index, call) in calls.iter().enumerate() {
            let action_id = Uuid::now_v7();
            let requested = Event::new(
                "action.requested",
                "habibi",
                root_trigger.correlation_id,
                Some(current_event.id),
                json!({
                    "batch_id": batch_id, "action_id": action_id, "index": index,
                    "tool_call_id": call.call_id, "tool": call.name,
                    "arguments": call.arguments, "model_log_id": model_log_id
                }),
            );
            self.append(&requested)?;
            request_event_ids.push(requested.id);
            let mut started = LogEntry::new(
                "debug",
                "action",
                "action.execution.started",
                root_trigger.correlation_id,
                Some(requested.id),
                root_trigger.correlation_id,
                json!({ "batch_id": batch_id, "action_id": action_id, "tool": call.name }),
            );
            started.batch_id = Some(batch_id.to_string());
            started.action_id = Some(action_id.to_string());
            started.tool_call_id = Some(call.call_id.clone());
            self.log(started)?;
            actions.push((index, action_id, call.clone(), requested));
        }

        let context = ToolContext {
            trigger: root_trigger.clone(),
            current_event: current_event.clone(),
            correlation_id: root_trigger.correlation_id,
        };
        let mut pending = FuturesUnordered::new();
        for (index, action_id, call, requested) in actions {
            let tools = self.tools.clone();
            let context = context.clone();
            pending.push(async move {
                let result = tools.execute(&call, &context).await;
                (index, action_id, call, requested, result)
            });
        }

        let mut ordered: Vec<Option<ActionResult>> = (0..calls.len()).map(|_| None).collect();
        let mut result_event_ids = vec![Uuid::nil(); calls.len()];
        while let Some((index, action_id, call, requested, execution)) = pending.next().await {
            let (output, result_event, level, log_payload) = match execution {
                Ok(execution) => {
                    let mut effect_ids = Vec::new();
                    for draft in execution.events {
                        validate_effect_namespace(&call.name, &draft.event_type)?;
                        let effect = Event::new(
                            draft.event_type,
                            format!("tool:{}", call.name),
                            root_trigger.correlation_id,
                            Some(requested.id),
                            draft.payload,
                        );
                        self.append(&effect)?;
                        effect_ids.push(effect.id);
                    }
                    let output = json!({ "ok": true, "result": execution.result });
                    let result_event = Event::new(
                        "action.result.succeeded",
                        "habibi",
                        root_trigger.correlation_id,
                        Some(requested.id),
                        json!({
                            "batch_id": batch_id, "action_id": action_id, "index": index,
                            "tool_call_id": call.call_id, "tool": call.name,
                            "result": execution.result, "effect_event_ids": effect_ids
                        }),
                    );
                    (
                        output,
                        result_event,
                        "info",
                        json!({ "effect_event_ids": effect_ids }),
                    )
                }
                Err(error) => {
                    let output = json!({ "ok": false, "error": error.to_string() });
                    let result_event = Event::new(
                        "action.result.failed",
                        "habibi",
                        root_trigger.correlation_id,
                        Some(requested.id),
                        json!({
                            "batch_id": batch_id, "action_id": action_id, "index": index,
                            "tool_call_id": call.call_id, "tool": call.name,
                            "error": { "message": error.to_string() }
                        }),
                    );
                    (
                        output,
                        result_event,
                        "error",
                        json!({ "error": error.to_string() }),
                    )
                }
            };
            self.append(&result_event)?;
            result_event_ids[index] = result_event.id;
            let mut completed_log = LogEntry::new(
                level,
                "action",
                "action.execution.completed",
                root_trigger.correlation_id,
                Some(result_event.id),
                root_trigger.correlation_id,
                log_payload,
            );
            completed_log.batch_id = Some(batch_id.to_string());
            completed_log.action_id = Some(action_id.to_string());
            completed_log.tool_call_id = Some(call.call_id.clone());
            self.log(completed_log)?;
            ordered[index] = Some(ActionResult { call, output });
        }

        let results = ordered
            .into_iter()
            .map(|result| result.context("action batch result missing"))
            .collect::<Result<Vec<_>>>()?;
        let completed_event = Event::new(
            "action.batch.completed",
            "habibi",
            root_trigger.correlation_id,
            Some(current_event.id),
            json!({
                "batch_id": batch_id,
                "root_trigger_event_id": root_trigger.id,
                "model_log_id": model_log_id,
                "action_request_event_ids": request_event_ids,
                "result_event_ids": result_event_ids,
                "results_in_call_order": result_event_ids
            }),
        );
        self.append(&completed_event)?;
        let mut completed_log = LogEntry::new(
            "info",
            "action",
            "action.batch.completed",
            root_trigger.correlation_id,
            Some(completed_event.id),
            root_trigger.correlation_id,
            json!({ "batch_id": batch_id, "completed_event_id": completed_event.id }),
        );
        completed_log.batch_id = Some(batch_id.to_string());
        self.log(completed_log)?;
        Ok(BatchExecution {
            results,
            completed_event,
        })
    }

    fn append(&self, event: &Event) -> Result<i64> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .append(event)
            .context("failed to append event")
    }

    fn log(&self, log: LogEntry) -> Result<i64> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .append_log(&log)
            .context("failed to append log")
    }
}

struct PendingModelEvent {
    event: Event,
    input: Vec<Value>,
}

struct ActionResult {
    call: ToolCall,
    output: Value,
}
struct BatchExecution {
    results: Vec<ActionResult>,
    completed_event: Event,
}

fn current_event_input(event: &Event) -> Result<Value> {
    Ok(json!({
        "role": "developer",
        "content": [{
            "type": "input_text",
            "text": format!("Current Habibi event being processed:\n{}", serde_json::to_string_pretty(event)?)
        }]
    }))
}

fn validate_effect_namespace(tool_name: &str, event_type: &str) -> Result<()> {
    if tool_name.starts_with("habibi.") {
        return Ok(());
    }
    let namespace = tool_name.split('.').next().unwrap_or_default();
    if !event_type.starts_with(&format!("{namespace}.")) {
        anyhow::bail!("tool '{tool_name}' cannot emit event type '{event_type}'");
    }
    Ok(())
}
