use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    event::{ConversationMessage, Event},
    model::ModelClient,
    store::SharedEventStore,
    tool::{ContinuationPolicy, ToolCall, ToolContext, ToolRuntime, domain_tool_name},
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

    pub fn record_runtime_started(&self) -> Result<()> {
        let correlation_id = Uuid::now_v7();
        self.append(&Event::new(
            "runtime.started",
            "habibi",
            correlation_id,
            None,
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
        let definitions = self.tools.definitions();
        let mut input = self.model.conversation_input(&conversation);
        let mut next_causation = trigger.id;

        for turn in 0..16 {
            let request = self.model.request_body(&input, &definitions);
            let invocation = Event::new(
                "model.invocation.started",
                "habibi",
                trigger.correlation_id,
                Some(next_causation),
                json!({
                    "provider": "openai-codex", "model": self.model.model_name(),
                    "endpoint": self.model.endpoint(), "trigger_event_id": trigger.id,
                    "turn": turn, "request": request
                }),
            );
            self.append(&invocation)?;

            let mut response = match self.model.invoke(request).await {
                Ok(response) => response,
                Err(error) => {
                    self.append(&Event::new(
                        "model.invocation.failed",
                        "habibi",
                        trigger.correlation_id,
                        Some(invocation.id),
                        json!({ "error": error.to_string(), "turn": turn }),
                    ))?;
                    return Err(error);
                }
            };

            for call in &mut response.tool_calls {
                call.name = domain_tool_name(&call.name, &definitions)
                    .with_context(|| format!("model called unknown tool '{}'", call.name))?
                    .to_owned();
            }

            let completed = Event::new(
                "model.invocation.completed",
                "habibi",
                trigger.correlation_id,
                Some(invocation.id),
                json!({
                    "provider": response.provider, "model": response.model,
                    "content": &response.content, "tool_calls": &response.tool_calls,
                    "output_items": &response.output_items, "usage": &response.usage,
                    "decision": if response.tool_calls.is_empty() { "settle" } else { "act" },
                    "turn": turn
                }),
            );
            self.append(&completed)?;

            if response.tool_calls.is_empty() {
                return Ok(());
            }

            input.extend(response.output_items.clone());
            let execution = self
                .execute_batch(trigger, &completed, &response.tool_calls)
                .await?;
            for result in &execution.results {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": result.call.call_id,
                    "output": serde_json::to_string(&result.output)?
                }));
            }
            if !execution.requires_continuation {
                return Ok(());
            }
            next_causation = execution.completed_event_id;
        }

        let failed = Event::new(
            "reaction.failed",
            "habibi",
            trigger.correlation_id,
            Some(next_causation),
            json!({ "error": "maximum model continuation turns exceeded", "max_turns": 16 }),
        );
        self.append(&failed)?;
        anyhow::bail!("maximum model continuation turns exceeded")
    }

    async fn execute_batch(
        &self,
        trigger: &Event,
        model_completed: &Event,
        calls: &[ToolCall],
    ) -> Result<BatchExecution> {
        let batch_id = Uuid::now_v7();
        let batch = Event::new(
            "action.batch.created",
            "habibi",
            trigger.correlation_id,
            Some(model_completed.id),
            json!({ "batch_id": batch_id, "call_count": calls.len(), "delivery": "after_all" }),
        );
        self.append(&batch)?;

        let mut actions = Vec::new();
        for (index, call) in calls.iter().enumerate() {
            let action_id = Uuid::now_v7();
            let policy = self
                .tools
                .continuation(&call.name)
                .unwrap_or(ContinuationPolicy::Required);
            let proposed = Event::new(
                "action.proposed",
                "habibi",
                trigger.correlation_id,
                Some(batch.id),
                json!({
                    "batch_id": batch_id, "action_id": action_id, "index": index,
                    "call_id": call.call_id, "tool": call.name, "arguments": call.arguments,
                    "continuation": policy
                }),
            );
            self.append(&proposed)?;
            let started = Event::new(
                "action.started",
                "habibi",
                trigger.correlation_id,
                Some(proposed.id),
                json!({ "batch_id": batch_id, "action_id": action_id, "call_id": call.call_id, "tool": call.name }),
            );
            self.append(&started)?;
            actions.push((index, action_id, call.clone(), policy, started));
        }

        let context = ToolContext {
            trigger: trigger.clone(),
            correlation_id: trigger.correlation_id,
        };
        let mut pending = FuturesUnordered::new();
        for (index, action_id, call, policy, started) in actions {
            let tools = self.tools.clone();
            let context = context.clone();
            pending.push(async move {
                let result = tools.execute(&call, &context).await;
                (index, action_id, call, policy, started, result)
            });
        }

        let mut ordered: Vec<Option<ActionResult>> = (0..calls.len()).map(|_| None).collect();
        let mut result_event_ids = Vec::new();
        while let Some((index, action_id, call, policy, started, execution)) = pending.next().await
        {
            let (output, terminal_event) = match execution {
                Ok(execution) => {
                    let mut effect_ids = Vec::new();
                    let mut cause = started.id;
                    for draft in execution.events {
                        validate_effect_namespace(&call.name, &draft.event_type)?;
                        let effect = Event::new(
                            draft.event_type,
                            format!("tool:{}", call.name),
                            trigger.correlation_id,
                            Some(started.id),
                            draft.payload,
                        );
                        self.append(&effect)?;
                        cause = effect.id;
                        effect_ids.push(effect.id);
                    }
                    let output = json!({ "ok": true, "result": execution.result });
                    let succeeded = Event::new(
                        "action.succeeded",
                        "habibi",
                        trigger.correlation_id,
                        Some(cause),
                        json!({
                            "batch_id": batch_id, "action_id": action_id, "call_id": call.call_id,
                            "tool": call.name, "result": execution.result, "effect_event_ids": effect_ids
                        }),
                    );
                    self.append(&succeeded)?;
                    (output, succeeded)
                }
                Err(error) => {
                    let output = json!({ "ok": false, "error": error.to_string() });
                    let failed = Event::new(
                        "action.failed",
                        "habibi",
                        trigger.correlation_id,
                        Some(started.id),
                        json!({
                            "batch_id": batch_id, "action_id": action_id, "call_id": call.call_id,
                            "tool": call.name, "error": error.to_string()
                        }),
                    );
                    self.append(&failed)?;
                    (output, failed)
                }
            };
            result_event_ids.push(terminal_event.id);
            ordered[index] = Some(ActionResult {
                call,
                policy,
                output,
            });
        }

        let results = ordered
            .into_iter()
            .map(|result| result.context("action batch result missing"))
            .collect::<Result<Vec<_>>>()?;
        let requires_continuation = results
            .iter()
            .any(|result| result.policy == ContinuationPolicy::Required);
        let completed = Event::new(
            "action.batch.completed",
            "habibi",
            trigger.correlation_id,
            Some(batch.id),
            json!({
                "batch_id": batch_id, "result_event_ids": result_event_ids,
                "requires_continuation": requires_continuation
            }),
        );
        self.append(&completed)?;
        Ok(BatchExecution {
            results,
            requires_continuation,
            completed_event_id: completed.id,
        })
    }

    fn append(&self, event: &Event) -> Result<i64> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .append(event)
            .context("failed to append event")
    }
}

struct ActionResult {
    call: ToolCall,
    policy: ContinuationPolicy,
    output: Value,
}

struct BatchExecution {
    results: Vec<ActionResult>,
    requires_continuation: bool,
    completed_event_id: Uuid,
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
