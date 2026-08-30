use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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

pub struct Engine {
    store: SharedEventStore,
    model: ModelClient,
    tools: Arc<ToolRuntime>,
}

impl Engine {
    pub fn new(store: SharedEventStore, model: ModelClient, tools: Arc<ToolRuntime>) -> Self {
        Self {
            store,
            model,
            tools,
        }
    }

    pub fn model_provider(&self) -> &'static str {
        self.model.provider_name()
    }

    pub fn model_name(&self) -> &str {
        self.model.model_name()
    }

    pub fn model_catalog(&self) -> Result<ModelCatalog> {
        self.model.catalog()
    }

    pub async fn refresh_model_catalog(&self) -> Result<ModelCatalog> {
        let catalog = self.model.refresh_catalog().await?;
        let dispatch_id = Uuid::now_v7();
        self.log(LogEntry::new(
            "info",
            "model",
            "model.catalog.refreshed",
            dispatch_id,
            None,
            dispatch_id,
            json!({
                "source": catalog.source,
                "updated_at": catalog.updated_at,
                "model_count": catalog.models.len()
            }),
        ))?;
        Ok(catalog)
    }

    pub fn record_runtime_started(&self) -> Result<()> {
        let dispatch_id = Uuid::now_v7();
        self.log(LogEntry::new(
            "info",
            "runtime",
            "runtime.started",
            dispatch_id,
            None,
            dispatch_id,
            json!({ "provider": self.model.provider_name(), "model": self.model.model_name() }),
        ))?;
        Ok(())
    }

    pub fn acquire_database_ownership(&self) -> Result<Uuid> {
        let owner_id = Uuid::now_v7();
        self.with_store(|store| store.acquire_engine_owner(owner_id))?;
        Ok(owner_id)
    }

    pub async fn run(
        self: Arc<Self>,
        owner_id: Uuid,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        if let Err(error) = self.with_store(|store| store.recover_processing(owner_id)) {
            eprintln!("engine recovery failed: {error:#}");
            let _ = self.with_store(|store| store.release_engine_owner(owner_id));
            return;
        }
        let ownership_lost = Arc::new(AtomicBool::new(false));
        let heartbeat_engine = self.clone();
        let heartbeat_lost = ownership_lost.clone();
        let mut heartbeat_shutdown = shutdown.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = heartbeat_shutdown.changed() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                }
                if heartbeat_engine
                    .with_store(|store| store.renew_engine_owner(owner_id))
                    .is_err()
                {
                    heartbeat_lost.store(true, Ordering::Release);
                    break;
                }
            }
        });
        loop {
            if *shutdown.borrow() || ownership_lost.load(Ordering::Acquire) {
                break;
            }
            let claimed = match self.with_store(|store| store.claim_next(owner_id)) {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("engine claim failed: {error:#}");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let Some(item) = claimed else {
                tokio::select! {
                    _ = shutdown.changed() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {},
                }
                continue;
            };
            let event_id = item.event.event.id;
            match self.process_claimed_event(&item.event.event).await {
                Ok(()) => {
                    if let Err(error) = self.with_store(|store| store.complete_inbox(event_id)) {
                        eprintln!("engine could not complete {event_id}: {error:#}");
                    }
                }
                Err(error) => {
                    let retry = item.attempts < 3;
                    if let Err(store_error) = self.with_store(|store| {
                        store.fail_inbox(event_id, &format!("{error:#}"), retry)
                    }) {
                        eprintln!(
                            "engine could not record failure for {event_id}: {store_error:#}"
                        );
                    }
                }
            }
        }
        heartbeat.abort();
        let _ = heartbeat.await;
        let _ = self.with_store(|store| store.release_engine_owner(owner_id));
    }

    async fn process_claimed_event(&self, current_event: &Event) -> Result<()> {
        let dispatch_id = current_event.id;
        self.log(LogEntry::new(
            "debug",
            "engine",
            "engine.dispatch.started",
            dispatch_id,
            Some(current_event.id),
            current_event.correlation_id,
            json!({ "event_id": current_event.id, "event_type": current_event.event_type }),
        ))?;

        let catalog = self.tools.catalog()?;
        let context_started = Instant::now();
        let mut extension_input = Vec::new();
        let mut context_hook_count = 0;
        for execution in self.tools.context_hooks(current_event)? {
            context_hook_count += 1;
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
                            "extension": execution.extension_id, "hook": execution.hook,
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
                        "extension": execution.extension_id, "hook": execution.hook,
                        "duration_ms": execution.duration_ms, "error": execution.error,
                    }),
                ),
                Err(error) => (
                    "warn",
                    "context.hook.failed",
                    json!({
                        "extension": execution.extension_id, "hook": execution.hook,
                        "duration_ms": execution.duration_ms, "error": error.to_string(),
                    }),
                ),
            };
            self.log(LogEntry::new(
                level,
                "context",
                name,
                dispatch_id,
                Some(current_event.id),
                current_event.correlation_id,
                payload,
            ))?;
        }
        let context_preparation_duration_ms = context_started.elapsed().as_millis();

        let mut suggestions = BTreeMap::new();
        for execution in self.tools.tool_suggestion_hooks(current_event)? {
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
                dispatch_id,
                Some(current_event.id),
                current_event.correlation_id,
                json!({ "extension": execution.extension_id, "hook": execution.hook,
                    "duration_ms": execution.duration_ms, "suggestions": execution.suggestions,
                    "error": execution.error }),
            ))?;
        }
        let mut tool_chain = ToolChainState::new(suggestions);
        tool_chain.observe_event(current_event);
        if current_event.event_type == "actions.completed"
            && let Some(result_ids) = current_event
                .payload
                .get("batched_result_event_ids")
                .and_then(Value::as_array)
        {
            for result_id in result_ids.iter().filter_map(Value::as_str) {
                if let Some(result) =
                    self.with_store(|store| store.get_event(Some(result_id), None))?
                {
                    tool_chain.observe_event(&result.event);
                }
            }
        }

        if let Some((state, completed)) =
            self.with_store(|store| store.action_group(current_event.id))?
        {
            if completed {
                return Ok(());
            }
            let group: DurableActionGroup = serde_json::from_value(state)?;
            anyhow::ensure!(
                group.catalog_generation == catalog.generation,
                "pinned tool catalog generation is unavailable after restart"
            );
            self.execute_persisted_action_group(current_event, &group, catalog)
                .await?;
            return Ok(());
        }
        if let Some(outcome) = self.with_store(|store| store.dispatch_outcome(current_event.id))? {
            let outcome: DurableDispatchOutcome = serde_json::from_value(outcome)?;
            let _exact_model_response = &outcome.model_response;
            anyhow::ensure!(
                outcome.catalog_generation == catalog.generation,
                "pinned tool catalog generation is unavailable after restart"
            );
            if !outcome.calls.is_empty() {
                self.execute_action_group(
                    current_event,
                    outcome.model_log_id,
                    &outcome.calls,
                    &outcome.advertised_tool_names,
                    catalog,
                )
                .await?;
            }
            return Ok(());
        }

        let rendering_started = Instant::now();
        let mut input = extension_input;
        input.push(current_event_input(&self.store, current_event)?);
        let input_bytes = serde_json::to_vec(&input)?.len();
        let context_log = LogEntry::new(
            "debug",
            "context",
            "context.compiled",
            dispatch_id,
            Some(current_event.id),
            current_event.correlation_id,
            json!({
                "current_event_id": current_event.id, "extension_hook_count": context_hook_count,
                "input": &input, "rendered_bytes": input_bytes,
                "estimated_tokens": input_bytes.div_ceil(4),
                "hook_preparation_duration_ms": context_preparation_duration_ms,
                "rendering_duration_ms": rendering_started.elapsed().as_millis(),
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
            dispatch_id,
            Some(current_event.id),
            current_event.correlation_id,
            json!({
                "current_event_id": current_event.id, "catalog_generation": catalog.generation,
                "duration_ms": surface_started.elapsed().as_millis(),
                "advertised": surface.definitions.len(), "advertised_schema_bytes": surface.advertised_schema_bytes,
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
            dispatch_id,
            Some(current_event.id),
            current_event.correlation_id,
            json!({
                "provider": self.model.provider_name(), "model": self.model.model_name(),
                "endpoint": self.model.endpoint(), "current_event_id": current_event.id,
                "current_event_type": current_event.event_type, "context_log_id": context_log_id,
                "tool_surface_log_id": surface_log_id, "tool_catalog_generation": catalog.generation,
                "request": request,
            }),
        );
        started_log.span_id = Some(model_span_id.clone());
        let started_log_id = started_log.id;
        self.log(started_log)?;

        let invocation_started_at = Instant::now();
        let mut response = match self.model.invoke(request, dispatch_id).await {
            Ok(response) => response,
            Err(error) => {
                let mut failed = LogEntry::new(
                    "error",
                    "model",
                    "model.invocation.failed",
                    dispatch_id,
                    Some(current_event.id),
                    current_event.correlation_id,
                    json!({ "error": error.to_string(), "started_log_id": started_log_id,
                        "current_event_type": current_event.event_type, "context_log_id": context_log_id,
                        "tool_surface_log_id": surface_log_id,
                        "duration_ms": invocation_started_at.elapsed().as_millis() }),
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
        let calls = plan_deliveries(response.tool_calls);
        tool_chain.observe_calls(
            &calls
                .iter()
                .map(|call| call.call.clone())
                .collect::<Vec<_>>(),
        );
        let estimated_cost = response
            .usage
            .as_ref()
            .and_then(|usage| self.model.estimate_cost(usage));
        let advertised_tool_names = surface
            .definitions
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let completed_payload = json!({
            "started_log_id": started_log_id, "provider": response.provider, "model": response.model,
            "current_event_type": current_event.event_type, "content": response.content,
            "tool_calls": calls, "output_items": response.output_items,
            "provider_response": response.provider_response, "usage": response.usage,
            "context_log_id": context_log_id, "tool_surface_log_id": surface_log_id,
            "tool_catalog_generation": catalog.generation, "estimated_cost": estimated_cost,
            "duration_ms": invocation_started_at.elapsed().as_millis(),
        });
        let mut completed_log = LogEntry::new(
            "info",
            "model",
            "model.invocation.completed",
            dispatch_id,
            Some(current_event.id),
            current_event.correlation_id,
            completed_payload.clone(),
        );
        completed_log.parent_span_id = Some(model_span_id);
        let completed_log_id = completed_log.id;
        let outcome = DurableDispatchOutcome {
            model_log_id: completed_log_id,
            catalog_generation: catalog.generation.clone(),
            advertised_tool_names: advertised_tool_names.clone(),
            calls: calls.clone(),
            model_response: completed_payload,
        };
        self.with_store(|store| {
            store.save_dispatch_outcome(
                current_event.id,
                &serde_json::to_value(&outcome)?,
                &completed_log,
            )
        })?;

        if !calls.is_empty() {
            self.execute_action_group(
                current_event,
                completed_log_id,
                &calls,
                &advertised_tool_names,
                catalog,
            )
            .await?;
        }
        self.log(LogEntry::new(
            "debug", "engine", "engine.dispatch.completed", dispatch_id,
            Some(current_event.id), current_event.correlation_id,
            json!({ "event_id": current_event.id, "outcome": if calls.is_empty() { "no_actions" } else { "actions_requested" } }),
        ))?;
        Ok(())
    }

    async fn execute_action_group(
        &self,
        current_event: &Event,
        model_log_id: Uuid,
        calls: &[PlannedCall],
        advertised_tool_names: &[String],
        catalog: Arc<ToolCatalog>,
    ) -> Result<()> {
        let group_id = Uuid::now_v7();
        let actions = calls
            .iter()
            .enumerate()
            .map(|(index, planned)| {
                let action_id = Uuid::now_v7();
                let requested = Event::new(
                    "action.requested",
                    "habibi",
                    current_event.correlation_id,
                    Some(current_event.id),
                    json!({
                        "group_id": group_id, "action_id": action_id, "index": index,
                        "tool_call_id": planned.call.call_id, "tool": planned.call.name,
                        "arguments": planned.call.arguments, "delivery": planned.delivery,
                        "model_log_id": model_log_id, "tool_catalog_generation": catalog.generation,
                        "advertised_tool_names": advertised_tool_names,
                    }),
                );
                DurableAction {
                    index,
                    action_id,
                    planned: planned.clone(),
                    requested,
                }
            })
            .collect::<Vec<_>>();
        let group = DurableActionGroup {
            group_id,
            model_log_id,
            catalog_generation: catalog.generation.clone(),
            advertised_tool_names: advertised_tool_names.to_vec(),
            actions,
        };
        self.with_store(|store| {
            store.create_action_group(
                current_event.id,
                &serde_json::to_value(&group)?,
                &group
                    .actions
                    .iter()
                    .map(|action| action.requested.clone())
                    .collect::<Vec<_>>(),
            )
        })?;
        let mut group_log = LogEntry::new(
            "info",
            "action",
            "actions.group.created",
            current_event.id,
            Some(current_event.id),
            current_event.correlation_id,
            json!({ "group_id": group_id, "call_count": calls.len(), "model_log_id": model_log_id,
                "tool_catalog_generation": catalog.generation }),
        );
        group_log.action_group_id = Some(group_id.to_string());
        self.log(group_log)?;
        self.execute_persisted_action_group(current_event, &group, catalog)
            .await
    }

    async fn execute_persisted_action_group(
        &self,
        current_event: &Event,
        group: &DurableActionGroup,
        catalog: Arc<ToolCatalog>,
    ) -> Result<()> {
        let completed_indices = self
            .with_store(|store| store.completed_action_indices(current_event.id))?
            .into_iter()
            .collect::<HashSet<_>>();
        let context = ToolContext {
            current_event: current_event.clone(),
            correlation_id: current_event.correlation_id,
        };
        let mut pending = FuturesUnordered::new();
        for action in group
            .actions
            .iter()
            .filter(|action| !completed_indices.contains(&action.index))
            .cloned()
        {
            let tools = self.tools.clone();
            let context = context.clone();
            let catalog = catalog.clone();
            let mut started = LogEntry::new(
                "debug",
                "action",
                "action.execution.started",
                current_event.id,
                Some(action.requested.id),
                current_event.correlation_id,
                json!({ "group_id": group.group_id, "action_id": action.action_id,
                    "tool": action.planned.call.name, "delivery": action.planned.delivery }),
            );
            started.action_group_id = Some(group.group_id.to_string());
            started.action_id = Some(action.action_id.to_string());
            started.tool_call_id = Some(action.planned.call.call_id.clone());
            self.log(started)?;
            pending.push(async move {
                let started = Instant::now();
                let result = tools
                    .execute(&catalog, &action.planned.call, &context)
                    .await;
                (action, started.elapsed().as_millis(), result)
            });
        }

        while let Some((action, duration_ms, execution)) = pending.next().await {
            let call = &action.planned.call;
            let mut effects = Vec::new();
            let (result_event, level, log_payload) = match execution {
                Ok(execution) => {
                    let prepared = prepare_effects(
                        &call.name,
                        current_event.correlation_id,
                        action.requested.id,
                        execution.host_events,
                        execution.events,
                    );
                    effects = prepared.events;
                    if let Some(message) = prepared.invalid_extension_effect {
                        let effect_ids = effects.iter().map(|effect| effect.id).collect::<Vec<_>>();
                        (
                            Event::new(
                                "action.result.failed",
                                "habibi",
                                current_event.correlation_id,
                                Some(action.requested.id),
                                json!({
                                    "group_id": group.group_id, "action_id": action.action_id, "index": action.index,
                                    "tool_call_id": call.call_id, "tool": call.name,
                                    "delivery": action.planned.delivery, "error": { "message": message },
                                    "effect_event_ids": effect_ids, "tool_catalog_generation": group.catalog_generation,
                                    "advertised_tool_names": group.advertised_tool_names,
                                }),
                            ),
                            "error",
                            json!({ "error": message, "effect_event_ids": effect_ids }),
                        )
                    } else {
                        let effect_ids = effects.iter().map(|effect| effect.id).collect::<Vec<_>>();
                        if let Some(error) = execution.failure {
                            (
                                Event::new(
                                    "action.result.failed",
                                    "habibi",
                                    current_event.correlation_id,
                                    Some(action.requested.id),
                                    json!({
                                        "group_id": group.group_id, "action_id": action.action_id, "index": action.index,
                                        "tool_call_id": call.call_id, "tool": call.name,
                                        "delivery": action.planned.delivery, "error": { "message": error },
                                        "effect_event_ids": effect_ids, "tool_catalog_generation": group.catalog_generation,
                                        "advertised_tool_names": group.advertised_tool_names,
                                    }),
                                ),
                                "error",
                                json!({ "error": error, "effect_event_ids": effect_ids }),
                            )
                        } else {
                            (
                                Event::new(
                                    "action.result.succeeded",
                                    "habibi",
                                    current_event.correlation_id,
                                    Some(action.requested.id),
                                    json!({
                                        "group_id": group.group_id, "action_id": action.action_id, "index": action.index,
                                        "tool_call_id": call.call_id, "tool": call.name,
                                        "delivery": action.planned.delivery, "result": execution.result,
                                        "effect_event_ids": effect_ids, "tool_catalog_generation": group.catalog_generation,
                                        "advertised_tool_names": group.advertised_tool_names,
                                    }),
                                ),
                                "info",
                                json!({ "effect_event_ids": effect_ids }),
                            )
                        }
                    }
                }
                Err(error) => (
                    Event::new(
                        "action.result.failed",
                        "habibi",
                        current_event.correlation_id,
                        Some(action.requested.id),
                        json!({
                            "group_id": group.group_id, "action_id": action.action_id, "index": action.index,
                            "tool_call_id": call.call_id, "tool": call.name,
                            "delivery": action.planned.delivery, "error": { "message": error.to_string() },
                            "tool_catalog_generation": group.catalog_generation,
                            "advertised_tool_names": group.advertised_tool_names,
                        }),
                    ),
                    "error",
                    json!({ "error": error.to_string() }),
                ),
            };
            let inserted = self.with_store(|store| {
                store.append_action_result(
                    current_event.id,
                    action.index,
                    &effects,
                    &result_event,
                    action.planned.delivery == DeliveryMode::Asap,
                )
            })?;
            if inserted {
                let mut completed_log = LogEntry::new(
                    level,
                    "action",
                    "action.execution.completed",
                    current_event.id,
                    Some(result_event.id),
                    current_event.correlation_id,
                    json!({ "tool": call.name, "duration_ms": duration_ms,
                        "delivery": action.planned.delivery, "tool_catalog_generation": group.catalog_generation,
                        "details": log_payload }),
                );
                completed_log.action_group_id = Some(group.group_id.to_string());
                completed_log.action_id = Some(action.action_id.to_string());
                completed_log.tool_call_id = Some(call.call_id.clone());
                self.log(completed_log)?;
            }
        }

        let result_event_ids = self
            .with_store(|store| store.action_result_ids(current_event.id, group.actions.len()))?;
        let request_event_ids = group
            .actions
            .iter()
            .map(|action| action.requested.id)
            .collect::<Vec<_>>();
        let asap_result_event_ids = group
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| action.planned.delivery == DeliveryMode::Asap)
            .map(|(index, _)| result_event_ids[index])
            .collect::<Vec<_>>();
        let batched_result_event_ids = group
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| action.planned.delivery == DeliveryMode::Batch)
            .map(|(index, _)| result_event_ids[index])
            .collect::<Vec<_>>();
        let completed_event = Event::new(
            "actions.completed",
            "habibi",
            current_event.correlation_id,
            Some(current_event.id),
            json!({
                "group_id": group.group_id, "model_log_id": group.model_log_id,
                "action_request_event_ids": request_event_ids, "result_event_ids": result_event_ids,
                "asap_result_event_ids": asap_result_event_ids,
                "batched_result_event_ids": batched_result_event_ids,
                "advertised_tool_names": group.advertised_tool_names,
            }),
        );
        self.with_store(|store| {
            store.complete_action_group(
                current_event.id,
                &completed_event,
                !batched_result_event_ids.is_empty(),
            )
        })?;
        let mut completed_log = LogEntry::new(
            "info",
            "action",
            "actions.group.completed",
            current_event.id,
            Some(completed_event.id),
            current_event.correlation_id,
            json!({ "group_id": group.group_id, "completed_event_id": completed_event.id,
                "asap_count": asap_result_event_ids.len(), "batched_count": batched_result_event_ids.len() }),
        );
        completed_log.action_group_id = Some(group.group_id.to_string());
        self.log(completed_log)?;
        Ok(())
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(&crate::store::EventStore) -> Result<T>,
    ) -> Result<T> {
        let store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        operation(&store)
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
            let definition = with_delivery_schema(
                catalog
                    .definition(&name)
                    .with_context(|| format!("tool candidate '{name}' is not registered"))?,
            );
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

    fn observe_event(&mut self, event: &Event) {
        if let Some(tools) = event
            .payload
            .get("advertised_tool_names")
            .and_then(Value::as_array)
        {
            for tool in tools.iter().filter_map(Value::as_str) {
                self.used.insert(tool.to_owned());
            }
        }
        let Some(tool) = event.payload.get("tool").and_then(Value::as_str) else {
            return;
        };
        self.used.insert(tool.to_owned());
        if tool == "habibi.tools.search"
            && let Some(found) = event
                .payload
                .pointer("/result/tools")
                .and_then(Value::as_array)
        {
            for tool in found {
                if let Some(name) = tool.get("tool").and_then(Value::as_str) {
                    self.discovered.insert(
                        name.to_owned(),
                        DiscoveredTool {
                            result_event_id: event.id,
                            advertised: false,
                            pruned_recorded: false,
                        },
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum DeliveryMode {
    Asap,
    Batch,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PlannedCall {
    #[serde(flatten)]
    call: ToolCall,
    delivery: DeliveryMode,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DurableDispatchOutcome {
    model_log_id: Uuid,
    catalog_generation: String,
    advertised_tool_names: Vec<String>,
    calls: Vec<PlannedCall>,
    model_response: Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DurableAction {
    index: usize,
    action_id: Uuid,
    planned: PlannedCall,
    requested: Event,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DurableActionGroup {
    group_id: Uuid,
    model_log_id: Uuid,
    catalog_generation: String,
    advertised_tool_names: Vec<String>,
    actions: Vec<DurableAction>,
}

fn plan_deliveries(calls: Vec<ToolCall>) -> Vec<PlannedCall> {
    let default = if calls.len() == 1 {
        DeliveryMode::Asap
    } else {
        DeliveryMode::Batch
    };
    calls
        .into_iter()
        .map(|mut call| {
            let delivery = call
                .arguments
                .as_object_mut()
                .and_then(|arguments| arguments.remove("_habibi_delivery"))
                .and_then(|value| match value.as_str() {
                    Some("asap") => Some(DeliveryMode::Asap),
                    Some("batch") => Some(DeliveryMode::Batch),
                    _ => None,
                })
                .unwrap_or(default);
            PlannedCall { call, delivery }
        })
        .collect()
}

fn with_delivery_schema(
    mut definition: crate::tool::ToolDefinition,
) -> crate::tool::ToolDefinition {
    if let Some(schema) = definition.input_schema.as_object_mut() {
        let properties = schema.entry("properties").or_insert_with(|| json!({}));
        if let Some(properties) = properties.as_object_mut() {
            properties.insert("_habibi_delivery".into(), json!({
                "type": "string", "enum": ["asap", "batch"],
                "description": "Habibi result delivery: immediately or with the completed action group"
            }));
        }
    }
    definition
}

struct PreparedEffects {
    events: Vec<Event>,
    invalid_extension_effect: Option<String>,
}

fn prepare_effects(
    tool_name: &str,
    correlation_id: Uuid,
    action_request_id: Uuid,
    host_events: Vec<crate::tool::HostEffect>,
    extension_events: Vec<crate::extension::EventDraft>,
) -> PreparedEffects {
    let mut events = host_events
        .into_iter()
        .map(|host_effect| {
            Event::new(
                host_effect.event.event_type,
                host_effect.source,
                correlation_id,
                Some(action_request_id),
                host_effect.event.payload,
            )
        })
        .collect::<Vec<_>>();
    if let Some(error) = extension_events
        .iter()
        .find_map(|draft| validate_effect_namespace(tool_name, &draft.event_type).err())
    {
        return PreparedEffects {
            events,
            invalid_extension_effect: Some(error.to_string()),
        };
    }
    events.extend(extension_events.into_iter().map(|draft| {
        Event::new(
            draft.event_type,
            format!("tool:{tool_name}"),
            correlation_id,
            Some(action_request_id),
            draft.payload,
        )
    }));
    PreparedEffects {
        events,
        invalid_extension_effect: None,
    }
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

    fn call(arguments: Value) -> ToolCall {
        ToolCall {
            call_id: "call".into(),
            name: "example.tool".into(),
            arguments,
        }
    }

    #[test]
    fn delivery_defaults_are_group_aware_and_metadata_is_stripped() {
        let one = plan_deliveries(vec![call(json!({ "value": 1 }))]);
        assert_eq!(one[0].delivery, DeliveryMode::Asap);
        let many = plan_deliveries(vec![
            call(json!({ "_habibi_delivery": "asap" })),
            call(json!({ "_habibi_delivery": "batch" })),
            call(json!({ "_habibi_delivery": "invalid" })),
        ]);
        assert_eq!(
            many.iter().map(|call| call.delivery).collect::<Vec<_>>(),
            vec![DeliveryMode::Asap, DeliveryMode::Batch, DeliveryMode::Batch]
        );
        assert!(
            many.iter()
                .all(|call| call.call.arguments.get("_habibi_delivery").is_none())
        );
    }

    #[test]
    fn delivery_metadata_is_advertised_without_becoming_required() {
        let definition = with_delivery_schema(crate::tool::ToolDefinition {
            name: "example.tool".into(),
            description: "Example".into(),
            input_schema: json!({ "type": "object", "additionalProperties": false,
                "properties": { "value": { "type": "string" } }, "required": ["value"] }),
        });
        assert_eq!(
            definition.input_schema["properties"]["_habibi_delivery"]["enum"],
            json!(["asap", "batch"])
        );
        assert_eq!(definition.input_schema["required"], json!(["value"]));
    }

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
    fn host_effects_survive_a_malformed_extension_effect() {
        let correlation_id = Uuid::now_v7();
        let action_request_id = Uuid::now_v7();
        let prepared = prepare_effects(
            "workspace.write",
            correlation_id,
            action_request_id,
            vec![crate::tool::HostEffect {
                source: "host:filesystem",
                event: crate::extension::EventDraft {
                    event_type: "workspace.file.written".into(),
                    payload: json!({ "path": "/workspace/note" }),
                    idempotency_key: None,
                },
            }],
            vec![crate::extension::EventDraft {
                event_type: "outside.invalid".into(),
                payload: json!({}),
                idempotency_key: None,
            }],
        );
        assert!(prepared.invalid_extension_effect.is_some());
        assert_eq!(prepared.events.len(), 1);
        assert_eq!(prepared.events[0].event_type, "workspace.file.written");
        assert_eq!(prepared.events[0].causation_id, Some(action_request_id));
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
