use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, Result};
use futures::{StreamExt, stream::FuturesUnordered};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    catalog::ModelCatalog,
    context::{compile_context_items, current_event_input},
    event::{Event, LogEntry},
    model::ModelClient,
    store::SharedEventStore,
    tool::{ToolCall, ToolCatalog, ToolContext, ToolRuntime, domain_tool_name},
};

pub struct Reactor {
    store: SharedEventStore,
    model: ModelClient,
    tools: Arc<ToolRuntime>,
}

impl Reactor {
    pub fn new(store: SharedEventStore, model: ModelClient, tools: Arc<ToolRuntime>) -> Self {
        Self {
            store,
            model,
            tools,
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

    pub async fn react(&self, trigger: &Event) -> Result<()> {
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

        let catalog = self.tools.catalog()?;
        let context_started = Instant::now();
        let context_hooks = self.tools.context_hooks(trigger)?;
        let mut extension_input = Vec::new();
        let mut context_hook_logs = Vec::new();
        for execution in context_hooks {
            let attempted = execution
                .contribution
                .as_ref()
                .map(|contribution| compile_context_items(&self.store, &contribution.items))
                .transpose();
            let (level, name, payload) = match attempted {
                Ok(Some(compiled)) => {
                    extension_input.extend(compiled.input);
                    (
                        "debug",
                        "context.hook.completed",
                        json!({
                            "extension": execution.extension_id,
                            "hook": execution.hook,
                            "duration_ms": execution.duration_ms,
                            "items_returned": execution.contribution.as_ref().map(|value| value.items.len()).unwrap_or(0),
                            "source_event_count": compiled.source_event_count,
                            "duplicate_items_omitted": compiled.duplicate_items_omitted,
                            "rendered_bytes": compiled.rendered_bytes,
                            "estimated_tokens": compiled.estimated_tokens,
                        }),
                    )
                }
                Ok(None) => (
                    "warn",
                    "context.hook.failed",
                    json!({
                        "extension": execution.extension_id,
                        "hook": execution.hook,
                        "duration_ms": execution.duration_ms,
                        "error": execution.error,
                    }),
                ),
                Err(error) => (
                    "warn",
                    "context.hook.failed",
                    json!({
                        "extension": execution.extension_id,
                        "hook": execution.hook,
                        "duration_ms": execution.duration_ms,
                        "error": error.to_string(),
                    }),
                ),
            };
            context_hook_logs.push(payload.clone());
            self.log(LogEntry::new(
                level,
                "context",
                name,
                reaction_id,
                Some(trigger.id),
                trigger.correlation_id,
                payload,
            ))?;
        }
        let context_preparation_duration_ms = context_started.elapsed().as_millis();
        let suggestion_hooks = self.tools.tool_suggestion_hooks(trigger)?;
        let mut suggestions = BTreeMap::new();
        for execution in suggestion_hooks {
            for suggestion in &execution.suggestions {
                suggestions
                    .entry(suggestion.tool.clone())
                    .or_insert_with(|| ToolCandidateOrigin::ExtensionSuggestion {
                        extension: execution.extension_id.clone(),
                        hook: execution.hook.clone(),
                        reason: suggestion.reason.clone(),
                    });
            }
            self.log(LogEntry::new(
                if execution.error.is_some() {
                    "warn"
                } else {
                    "debug"
                },
                "tool",
                if execution.error.is_some() {
                    "tool.suggestion_hook.failed"
                } else {
                    "tool.suggestion_hook.completed"
                },
                reaction_id,
                Some(trigger.id),
                trigger.correlation_id,
                json!({
                    "extension": execution.extension_id,
                    "hook": execution.hook,
                    "duration_ms": execution.duration_ms,
                    "suggestions": execution.suggestions,
                    "error": execution.error,
                }),
            ))?;
        }
        let mut tool_chain = ToolChainState::new(suggestions);
        let mut queue = VecDeque::from([trigger.clone()]);
        let mut processed_event_ids = Vec::new();
        let mut final_event_id = trigger.id;

        while let Some(current_event) = queue.pop_front() {
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

            let context_rendering_started = Instant::now();
            let mut input = extension_input.clone();
            input.push(current_event_input(&self.store, &current_event)?);
            let input_bytes = serde_json::to_vec(&input)?.len();
            let context_log = LogEntry::new(
                "debug",
                "context",
                "context.compiled",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "root_trigger_event_id": trigger.id,
                    "current_event_id": current_event.id,
                    "extension_hook_count": context_hook_logs.len(),
                    "extension_items": extension_input.len(),
                    "input": &input,
                    "rendered_bytes": input_bytes,
                    "estimated_tokens": input_bytes.div_ceil(4),
                    "hook_preparation_duration_ms": context_preparation_duration_ms,
                    "rendering_duration_ms": context_rendering_started.elapsed().as_millis(),
                }),
            );
            let context_log_id = context_log.id;
            self.log(context_log)?;

            let surface_started = Instant::now();
            let surface = tool_chain.prepare_surface(&catalog)?;
            let surface_log = LogEntry::new(
                "debug",
                "tool",
                "tool.surface.prepared",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "root_trigger_event_id": trigger.id,
                    "current_event_id": current_event.id,
                    "catalog_generation": catalog.generation,
                    "invocation_index": tool_chain.invocation_index,
                    "duration_ms": surface_started.elapsed().as_millis(),
                    "advertised": surface.definitions.len(),
                    "pruned": surface.records.iter().filter(|record| record.decision == "pruned_unused").count(),
                    "advertised_schema_bytes": surface.advertised_schema_bytes,
                    "estimated_advertised_schema_tokens": surface.advertised_schema_bytes.div_ceil(4),
                    "tools": surface.records,
                }),
            );
            let surface_log_id = surface_log.id;
            self.log(surface_log)?;

            let request = self.model.request_body(&input, &surface.definitions);
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
                    "current_event_id": current_event.id,
                    "current_event_type": current_event.event_type,
                    "context_log_id": context_log_id,
                    "tool_surface_log_id": surface_log_id,
                    "tool_catalog_generation": catalog.generation, "request": request
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
                            "current_event_type": current_event.event_type,
                            "context_log_id": context_log_id, "tool_surface_log_id": surface_log_id,
                            "duration_ms": invocation_started_at.elapsed().as_millis()
                        }),
                    );
                    failed.parent_span_id = Some(model_span_id);
                    self.log(failed)?;
                    return Err(error);
                }
            };
            for call in &mut response.tool_calls {
                call.name = domain_tool_name(&call.name, &surface.definitions)
                    .with_context(|| {
                        format!("model called tool '{}' that was not advertised", call.name)
                    })?
                    .to_owned();
            }
            tool_chain.observe_calls(&response.tool_calls);

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
                    "model": response.model, "current_event_type": current_event.event_type,
                    "content": &response.content,
                    "tool_calls": &response.tool_calls, "output_items": &response.output_items,
                    "provider_response": &response.provider_response,
                    "usage": &response.usage, "context_log_id": context_log_id,
                    "tool_surface_log_id": surface_log_id,
                    "tool_catalog_generation": catalog.generation,
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
                    catalog.clone(),
                )
                .await?;
            tool_chain.observe_results(&batch.results);
            self.log(LogEntry::new(
                "debug",
                "reactor",
                "event.processing.completed",
                reaction_id,
                Some(current_event.id),
                trigger.correlation_id,
                json!({
                    "event_id": current_event.id,
                    "outcome": if batch.settles_chain { "terminal_action" } else { "actions_requested" },
                    "next_event_id": batch.completed_event.id
                }),
            ))?;
            if !batch.settles_chain {
                queue.push_back(batch.completed_event);
            }
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
                "processed_model_event_ids": processed_event_ids,
                "tool_usage": tool_chain.usage,
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
        catalog: Arc<ToolCatalog>,
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
                    "arguments": call.arguments, "model_log_id": model_log_id,
                    "tool_catalog_generation": catalog.generation
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
            let catalog = catalog.clone();
            pending.push(async move {
                let started = Instant::now();
                let result = tools.execute(&catalog, &call, &context).await;
                (
                    index,
                    action_id,
                    call,
                    requested,
                    started.elapsed().as_millis(),
                    result,
                )
            });
        }

        let mut ordered: Vec<Option<ActionResult>> = (0..calls.len()).map(|_| None).collect();
        let mut result_event_ids = vec![Uuid::nil(); calls.len()];
        while let Some((index, action_id, call, requested, duration_ms, execution)) =
            pending.next().await
        {
            let (output, result_event, level, log_payload, settle) = match execution {
                Ok(execution) => {
                    let settle = execution.settle;
                    let mut effect_ids = Vec::new();
                    for host_effect in execution.host_events {
                        let effect = Event::new(
                            host_effect.event.event_type,
                            host_effect.source,
                            root_trigger.correlation_id,
                            Some(requested.id),
                            host_effect.event.payload,
                        );
                        self.append(&effect)?;
                        effect_ids.push(effect.id);
                    }
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
                    if let Some(error) = execution.failure {
                        let output = json!({ "ok": false, "error": error });
                        let result_event = Event::new(
                            "action.result.failed",
                            "habibi",
                            root_trigger.correlation_id,
                            Some(requested.id),
                            json!({
                                "batch_id": batch_id, "action_id": action_id, "index": index,
                                "tool_call_id": call.call_id, "tool": call.name,
                                "error": { "message": error }, "effect_event_ids": effect_ids,
                                "tool_catalog_generation": catalog.generation
                            }),
                        );
                        (
                            output,
                            result_event,
                            "error",
                            json!({ "error": error, "effect_event_ids": effect_ids }),
                            false,
                        )
                    } else {
                        let output = json!({ "ok": true, "result": execution.result });
                        let result_event = Event::new(
                            "action.result.succeeded",
                            "habibi",
                            root_trigger.correlation_id,
                            Some(requested.id),
                            json!({
                                "batch_id": batch_id, "action_id": action_id, "index": index,
                                "tool_call_id": call.call_id, "tool": call.name,
                                "result": execution.result, "effect_event_ids": effect_ids,
                                "tool_catalog_generation": catalog.generation
                            }),
                        );
                        (
                            output,
                            result_event,
                            "info",
                            json!({ "effect_event_ids": effect_ids }),
                            settle,
                        )
                    }
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
                            "error": { "message": error.to_string() },
                            "tool_catalog_generation": catalog.generation
                        }),
                    );
                    (
                        output,
                        result_event,
                        "error",
                        json!({ "error": error.to_string() }),
                        false,
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
                json!({
                    "tool": call.name,
                    "duration_ms": duration_ms,
                    "tool_catalog_generation": catalog.generation,
                    "details": log_payload,
                }),
            );
            completed_log.batch_id = Some(batch_id.to_string());
            completed_log.action_id = Some(action_id.to_string());
            completed_log.tool_call_id = Some(call.call_id.clone());
            self.log(completed_log)?;
            ordered[index] = Some(ActionResult {
                call,
                output,
                result_event_id: result_event.id,
                settle,
            });
        }

        let results = ordered
            .into_iter()
            .map(|result| result.context("action batch result missing"))
            .collect::<Result<Vec<_>>>()?;
        let settles_chain = results.iter().any(|result| result.settle);
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
            settles_chain,
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum ToolCandidateOrigin {
    Core,
    ExtensionSuggestion {
        extension: String,
        hook: String,
        reason: Option<String>,
    },
    ToolSearch {
        result_event_id: Uuid,
    },
    UsedEarlier,
}

#[derive(Debug, Clone)]
struct DiscoveredTool {
    result_event_id: Uuid,
    advertised: bool,
    pruned_recorded: bool,
}

#[derive(Debug, Default, serde::Serialize)]
struct ToolUsage {
    advertised: u64,
    called: u64,
    succeeded: u64,
    failed: u64,
    estimated_schema_tokens: u64,
}

struct ToolChainState {
    suggestions: BTreeMap<String, ToolCandidateOrigin>,
    discovered: BTreeMap<String, DiscoveredTool>,
    used: HashSet<String>,
    usage: BTreeMap<String, ToolUsage>,
    invocation_index: u64,
}

#[derive(serde::Serialize)]
struct ToolSurfaceRecord {
    tool: String,
    origin: ToolCandidateOrigin,
    decision: String,
    schema_bytes: usize,
    estimated_schema_tokens: usize,
}

struct ToolSurface {
    definitions: Vec<crate::tool::ToolDefinition>,
    records: Vec<ToolSurfaceRecord>,
    advertised_schema_bytes: usize,
}

impl ToolChainState {
    fn new(suggestions: BTreeMap<String, ToolCandidateOrigin>) -> Self {
        Self {
            suggestions,
            discovered: BTreeMap::new(),
            used: HashSet::new(),
            usage: BTreeMap::new(),
            invocation_index: 0,
        }
    }

    fn prepare_surface(&mut self, catalog: &ToolCatalog) -> Result<ToolSurface> {
        self.invocation_index += 1;
        let mut candidates =
            BTreeMap::from([("habibi.tools.search".to_owned(), ToolCandidateOrigin::Core)]);
        candidates.extend(self.suggestions.clone());
        for tool in &self.used {
            candidates
                .entry(tool.clone())
                .or_insert(ToolCandidateOrigin::UsedEarlier);
        }
        for (tool, discovery) in &self.discovered {
            if !discovery.advertised || self.used.contains(tool) {
                candidates.insert(
                    tool.clone(),
                    ToolCandidateOrigin::ToolSearch {
                        result_event_id: discovery.result_event_id,
                    },
                );
            }
        }

        let mut definitions = Vec::new();
        let mut records = Vec::new();
        let mut advertised_schema_bytes = 0;
        for (name, origin) in candidates {
            let definition = catalog
                .definition(&name)
                .with_context(|| format!("tool candidate '{name}' is not registered"))?;
            let schema_bytes = serde_json::to_vec(&definition)?.len();
            advertised_schema_bytes += schema_bytes;
            let estimated_schema_tokens = schema_bytes.div_ceil(4);
            let usage = self.usage.entry(name.clone()).or_default();
            usage.advertised += 1;
            usage.estimated_schema_tokens += estimated_schema_tokens as u64;
            records.push(ToolSurfaceRecord {
                tool: name,
                origin,
                decision: "advertised".into(),
                schema_bytes,
                estimated_schema_tokens,
            });
            definitions.push(definition);
        }
        for (name, discovery) in &mut self.discovered {
            if !discovery.advertised {
                discovery.advertised = true;
            } else if !discovery.pruned_recorded
                && !self.used.contains(name)
                && !records.iter().any(|record| record.tool == *name)
            {
                discovery.pruned_recorded = true;
                records.push(ToolSurfaceRecord {
                    tool: name.clone(),
                    origin: ToolCandidateOrigin::ToolSearch {
                        result_event_id: discovery.result_event_id,
                    },
                    decision: "pruned_unused".into(),
                    schema_bytes: 0,
                    estimated_schema_tokens: 0,
                });
            }
        }
        Ok(ToolSurface {
            definitions,
            records,
            advertised_schema_bytes,
        })
    }

    fn observe_calls(&mut self, calls: &[ToolCall]) {
        for call in calls {
            self.used.insert(call.name.clone());
            self.usage.entry(call.name.clone()).or_default().called += 1;
        }
    }

    fn observe_results(&mut self, results: &[ActionResult]) {
        for result in results {
            let usage = self.usage.entry(result.call.name.clone()).or_default();
            if result.output.get("ok").and_then(Value::as_bool) == Some(true) {
                usage.succeeded += 1;
            } else {
                usage.failed += 1;
            }
            if result.call.name == "habibi.tools.search"
                && let Some(found) = result
                    .output
                    .pointer("/result/tools")
                    .and_then(Value::as_array)
            {
                for tool in found {
                    if let Some(name) = tool.get("tool").and_then(Value::as_str) {
                        self.discovered.insert(
                            name.to_owned(),
                            DiscoveredTool {
                                result_event_id: result.result_event_id,
                                advertised: false,
                                pruned_recorded: false,
                            },
                        );
                    }
                }
            }
        }
    }
}

struct ActionResult {
    call: ToolCall,
    output: Value,
    result_event_id: Uuid,
    settle: bool,
}
struct BatchExecution {
    results: Vec<ActionResult>,
    completed_event: Event,
    settles_chain: bool,
}

fn validate_effect_namespace(tool_name: &str, event_type: &str) -> Result<()> {
    if event_type.starts_with("workspace.") {
        anyhow::bail!("workspace effect events can only be emitted by the filesystem host");
    }
    if event_type.starts_with("process.") {
        anyhow::bail!("process effect events can only be emitted by the process host");
    }
    if tool_name.starts_with("habibi.") {
        return Ok(());
    }
    let namespace = tool_name.split('.').next().unwrap_or_default();
    if !event_type.starts_with(&format!("{namespace}.")) {
        anyhow::bail!("tool '{tool_name}' cannot emit event type '{event_type}'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{extension::ExtensionManager, store::EventStore};

    #[test]
    fn rejects_lua_authored_process_effects() {
        assert!(
            validate_effect_namespace("process.run", "process.execution.completed")
                .unwrap_err()
                .to_string()
                .contains("process host")
        );
    }

    #[test]
    fn rejects_lua_authored_workspace_effects() {
        assert!(
            validate_effect_namespace("workspace.write", "workspace.file.written")
                .unwrap_err()
                .to_string()
                .contains("filesystem host")
        );
    }

    #[test]
    fn prunes_searched_but_unused_tools_after_one_advertisement() {
        let directory = tempfile::tempdir().unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extensions = Arc::new(ExtensionManager::load(directory.path(), store.clone()).unwrap());
        let runtime = ToolRuntime::new(store, extensions).unwrap();
        let catalog = runtime.catalog().unwrap();
        let mut chain = ToolChainState::new(BTreeMap::new());
        chain.discovered.insert(
            "habibi.events.get".into(),
            DiscoveredTool {
                result_event_id: Uuid::now_v7(),
                advertised: false,
                pruned_recorded: false,
            },
        );
        let first = chain.prepare_surface(&catalog).unwrap();
        assert!(
            first
                .definitions
                .iter()
                .any(|definition| definition.name == "habibi.events.get")
        );
        let second = chain.prepare_surface(&catalog).unwrap();
        assert!(
            !second
                .definitions
                .iter()
                .any(|definition| definition.name == "habibi.events.get")
        );
        assert!(second.records.iter().any(|record| {
            record.tool == "habibi.events.get" && record.decision == "pruned_unused"
        }));
        let third = chain.prepare_surface(&catalog).unwrap();
        assert!(!third.records.iter().any(|record| {
            record.tool == "habibi.events.get" && record.decision == "pruned_unused"
        }));
    }
}
