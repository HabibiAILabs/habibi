use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    convert::Infallible,
    path::PathBuf,
    sync::Arc,
    time::{Duration as StdDuration, Instant},
};

use anyhow::Context;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{any, get, post, put},
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use crate::process::normalize_executable_grants;

use crate::{
    engine::Engine,
    event::Event,
    extension::{ExtensionManager, RequestData, RouteOutcome},
    filesystem::normalize_grant_roots,
    installer::ExtensionInstaller,
    store::{EventStore, EventTailQuery, SharedEventStore, StoreEventQuery, StoreLogQuery},
    studio::{
        CreateDraftDirectoryRequest, CreateDraftRequest, DraftFileRequest, StudioService,
        WriteDraftFileRequest,
    },
};

#[derive(Clone)]
pub struct WebState {
    pub extensions: Arc<ExtensionManager>,
    pub engine: Arc<Engine>,
    pub store: SharedEventStore,
    pub extensions_dir: PathBuf,
    pub studio: Arc<StudioService>,
    pub local_admin: bool,
    pub catalog_mutation_lock: Arc<tokio::sync::Mutex<()>>,
}

pub fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(home_page))
        .route("/extensions", get(extensions_page))
        .route("/events", get(events_page))
        .route("/logs", get(logs_page))
        .route("/trace", get(trace_page))
        .route("/stats", get(stats_page))
        .route("/studio", get(studio_page))
        .route("/assets/habibi-logo.svg", get(logo_asset))
        .route("/assets/core.css", get(core_css_asset))
        .route("/assets/extensions.js", get(extensions_js_asset))
        .route("/assets/events.js", get(events_js_asset))
        .route("/assets/logs.js", get(logs_js_asset))
        .route("/assets/trace.js", get(trace_js_asset))
        .route("/assets/graph-layout.mjs", get(graph_layout_js_asset))
        .route(
            "/assets/memory-graph-state.mjs",
            get(memory_graph_state_js_asset),
        )
        .route("/assets/memory-graph.js", get(memory_graph_js_asset))
        .route("/assets/vgpu-LICENSE.txt", get(vgpu_license_asset))
        .route("/assets/markdown.js", get(markdown_js_asset))
        .route("/assets/stats.js", get(stats_js_asset))
        .route("/assets/studio.js", get(studio_js_asset))
        .route("/api/events", get(list_events))
        .route("/api/event-graph", get(event_graph))
        .route("/api/events/stream", get(stream_events))
        .route("/api/logs", get(list_logs))
        .route("/api/trace", get(trace))
        .route("/api/stats", get(stats))
        .route("/api/models", get(models))
        .route("/api/models/refresh", post(refresh_models))
        .route("/api/extensions", get(list_extensions))
        .route("/api/studio/drafts", get(list_drafts).post(create_draft))
        .route("/api/studio/drafts/{draft_id}/files", get(list_draft_files))
        .route(
            "/api/studio/drafts/{draft_id}/files/{*path}",
            get(read_draft_file).put(write_draft_file),
        )
        .route(
            "/api/studio/drafts/{draft_id}/directories",
            post(create_draft_directory),
        )
        .route(
            "/api/studio/drafts/{draft_id}/validate",
            post(validate_draft),
        )
        .route("/api/studio/drafts/{draft_id}/install", post(install_draft))
        .route("/api/extensions/{extension_id}", put(toggle_extension))
        .route(
            "/api/extensions/{extension_id}/grants",
            get(extension_grants).put(update_extension_grants),
        )
        .route(
            "/api/extensions/{extension_id}/check-update",
            post(check_extension_update),
        )
        .route(
            "/api/extensions/{extension_id}/update",
            post(update_extension),
        )
        .route(
            "/api/extensions/{extension_id}/reload",
            post(reload_extension),
        )
        .route("/extensions/{extension_id}", any(extension_root))
        .route("/extensions/{extension_id}/", any(extension_root))
        .route("/extensions/{extension_id}/{*path}", any(extension_path))
        .with_state(state)
}

async fn home_page() -> Response {
    html_response(include_str!("../web/home.html"))
}

async fn extensions_page() -> Response {
    html_response(include_str!("../web/extensions.html"))
}

async fn events_page() -> Response {
    html_response(include_str!("../web/events.html"))
}

async fn logs_page() -> Response {
    html_response(include_str!("../web/logs.html"))
}

async fn trace_page() -> Response {
    html_response(include_str!("../web/trace.html"))
}

async fn stats_page() -> Response {
    html_response(include_str!("../web/stats.html"))
}

async fn studio_page() -> Response {
    html_response(include_str!("../web/studio.html"))
}

async fn logo_asset() -> Response {
    asset_response("image/svg+xml", include_bytes!("../web/habibi-logo.svg"))
}

async fn core_css_asset() -> Response {
    asset_response("text/css; charset=utf-8", include_bytes!("../web/core.css"))
}

async fn extensions_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/extensions.js"),
    )
}

async fn stats_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/stats.js"),
    )
}

async fn studio_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/studio.js"),
    )
}

async fn logs_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/logs.js"),
    )
}

async fn trace_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/trace.js"),
    )
}

async fn graph_layout_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/graph-layout.mjs"),
    )
}

async fn memory_graph_state_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/memory-graph-state.mjs"),
    )
}

async fn memory_graph_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/generated/memory-graph.js"),
    )
}

async fn vgpu_license_asset() -> Response {
    asset_response(
        "text/plain; charset=utf-8",
        include_bytes!("../web/vgpu-LICENSE.txt"),
    )
}

async fn markdown_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/markdown.js"),
    )
}

async fn events_js_asset() -> Response {
    asset_response(
        "text/javascript; charset=utf-8",
        include_bytes!("../web/events.js"),
    )
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    #[serde(rename = "type")]
    event_type: Option<String>,
    prefix: Option<String>,
    source: Option<String>,
    event_id: Option<String>,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    before_sequence: Option<i64>,
    after_sequence: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    window: Option<String>,
    payload_contains: Option<String>,
    limit: Option<usize>,
}

async fn list_events(State(state): State<WebState>, Query(query): Query<EventsQuery>) -> Response {
    match build_event_query(query).and_then(|query| {
        state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .query_events(&query)
    }) {
        Ok(events) => json_response(StatusCode::OK, json!({ "events": events })),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct EventGraphQuery {
    #[serde(rename = "type")]
    event_type: Option<String>,
    source: Option<String>,
    correlation_id: Option<String>,
    limit: Option<usize>,
}

async fn event_graph(
    State(state): State<WebState>,
    Query(query): Query<EventGraphQuery>,
) -> Response {
    let result: anyhow::Result<Value> = (|| {
        let (event_query, limit) = build_event_graph_query(query)?;
        let locked = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        query_event_graph(&locked, &event_query, limit)
    })();
    match result {
        Ok(graph) => json_response(StatusCode::OK, graph),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

fn build_event_graph_query(query: EventGraphQuery) -> anyhow::Result<(StoreEventQuery, usize)> {
    let correlation_id = query
        .correlation_id
        .filter(|value| !value.is_empty())
        .map(|value| Uuid::parse_str(&value))
        .transpose()?;
    let limit = query.limit.unwrap_or(250).clamp(1, 1_000);
    Ok((
        StoreEventQuery {
            event_type: query.event_type.filter(|value| !value.is_empty()),
            source: query.source.filter(|value| !value.is_empty()),
            correlation_id,
            limit: limit + 1,
            ..StoreEventQuery::default()
        },
        limit,
    ))
}

fn query_event_graph(
    store: &EventStore,
    event_query: &StoreEventQuery,
    limit: usize,
) -> anyhow::Result<Value> {
    let cursor = store.latest_event_sequence()?;
    let mut events = store.query_events(event_query)?;
    let events_truncated = events.len() > limit;
    if events_truncated {
        events.remove(0);
    }
    let event_ids = events
        .iter()
        .map(|stored| stored.event.id)
        .collect::<Vec<_>>();
    let mut links = store.event_links_for_events(&event_ids, 2_001)?;
    let links_truncated = links.len() > 2_000;
    if links_truncated {
        links.pop();
    }
    Ok(build_event_graph_response(
        events,
        links,
        events_truncated,
        links_truncated,
        cursor,
    ))
}

fn build_event_graph_response(
    events: Vec<crate::event::StoredEvent>,
    links: Vec<Value>,
    events_truncated: bool,
    links_truncated: bool,
    cursor: i64,
) -> Value {
    json!({
        "events": events,
        "links": links,
        "events_truncated": events_truncated,
        "links_truncated": links_truncated,
        "cursor": cursor,
    })
}

#[derive(Debug, Deserialize)]
struct EventStreamQuery {
    #[serde(rename = "type")]
    event_types: Option<String>,
    exact_type: Option<String>,
    prefix: Option<String>,
    correlation_id: Option<String>,
    after_sequence: Option<String>,
}

struct EventStreamState {
    store: SharedEventStore,
    query: EventTailQuery,
    initial_cursor: Option<i64>,
    pending: VecDeque<crate::event::StoredEvent>,
    heartbeat_at: Instant,
}

fn parse_stream_event_types(value: Option<&str>) -> anyhow::Result<Vec<String>> {
    let types = value
        .map(|types| {
            types
                .split(',')
                .map(str::trim)
                .filter(|event_type| !event_type.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    anyhow::ensure!(
        value.is_none() || !types.is_empty(),
        "type must contain an exact event type"
    );
    Ok(types)
}

fn parse_stream_exact_type(value: Option<&str>) -> anyhow::Result<Option<String>> {
    let exact_type = value.filter(|event_type| !event_type.is_empty());
    anyhow::ensure!(
        value.is_none() || exact_type.is_some(),
        "exact_type must contain one exact event type"
    );
    Ok(exact_type.map(str::to_owned))
}

fn parse_stream_cursor(
    explicit: Option<&str>,
    last_event_id: Option<&axum::http::HeaderValue>,
    fallback: i64,
) -> anyhow::Result<i64> {
    let cursor = if let Some(last_event_id) = last_event_id {
        Some(last_event_id.to_str()?)
    } else {
        explicit
    };
    let Some(cursor) = cursor else {
        return Ok(fallback);
    };
    let cursor = cursor
        .parse::<i64>()
        .context("after_sequence/Last-Event-ID must be a non-negative integer")?;
    anyhow::ensure!(
        cursor >= 0,
        "after_sequence/Last-Event-ID must be a non-negative integer"
    );
    Ok(cursor)
}

fn sse_cursor_frame(sequence: i64) -> String {
    format!("id: {sequence}\nevent: habibi.cursor\ndata: {{\"sequence\":{sequence}}}\n\n")
}

fn sse_event_frame(sequence: i64, data: &str) -> String {
    format!("id: {sequence}\nevent: habibi.event\ndata: {data}\n\n")
}

async fn stream_events(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(query): Query<EventStreamQuery>,
) -> Response {
    let parsed = (|| -> anyhow::Result<EventStreamState> {
        let event_types = parse_stream_event_types(query.event_types.as_deref())?;
        let exact_type = parse_stream_exact_type(query.exact_type.as_deref())?;
        let correlation_id = query
            .correlation_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()?;
        let fallback = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .latest_event_sequence()?;
        let after_sequence = parse_stream_cursor(
            query.after_sequence.as_deref(),
            headers.get("last-event-id"),
            fallback,
        )?;
        Ok(EventStreamState {
            store: state.store.clone(),
            query: EventTailQuery {
                event_types,
                event_type: exact_type,
                event_type_prefix: query.prefix,
                correlation_id,
                after_sequence,
                limit: 200,
            },
            initial_cursor: Some(after_sequence),
            pending: VecDeque::new(),
            heartbeat_at: Instant::now() + StdDuration::from_secs(15),
        })
    })();
    let stream_state = match parsed {
        Ok(state) => state,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            );
        }
    };
    let stream = futures::stream::unfold(stream_state, |mut state| async move {
        loop {
            if let Some(cursor) = state.initial_cursor.take() {
                return Some((Ok(Bytes::from(sse_cursor_frame(cursor))), state));
            }
            if let Some(event) = state.pending.pop_front() {
                state.query.after_sequence = event.sequence;
                let data = match serde_json::to_string(&event) {
                    Ok(data) => data,
                    Err(error) => {
                        return Some((
                            Ok::<Bytes, Infallible>(Bytes::from(format!(
                                ": serialization error: {error}\n\n"
                            ))),
                            state,
                        ));
                    }
                };
                return Some((
                    Ok(Bytes::from(sse_event_frame(event.sequence, &data))),
                    state,
                ));
            }
            let events = match state.store.lock() {
                Ok(store) => store.query_event_tail(&state.query),
                Err(_) => Err(anyhow::anyhow!("event store lock poisoned")),
            };
            match events {
                Ok(events) if !events.is_empty() => {
                    state.pending = events.into();
                    continue;
                }
                Err(error) => {
                    tokio::time::sleep(StdDuration::from_secs(1)).await;
                    return Some((
                        Ok(Bytes::from(format!(": store error: {error}\n\n"))),
                        state,
                    ));
                }
                _ => {}
            }
            tokio::time::sleep(StdDuration::from_millis(500)).await;
            if Instant::now() >= state.heartbeat_at {
                state.heartbeat_at = Instant::now() + StdDuration::from_secs(15);
                return Some((Ok(Bytes::from_static(b": heartbeat\n\n")), state));
            }
        }
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .expect("valid SSE response")
}

#[derive(Debug, Deserialize)]
struct TraceQuery {
    event_id: Option<String>,
    correlation_id: Option<String>,
}

async fn trace(State(state): State<WebState>, Query(query): Query<TraceQuery>) -> Response {
    let result = (|| {
        let locked = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        let focus = query.event_id.as_deref().map(Uuid::parse_str).transpose()?;
        let correlation = if let Some(event_id) = focus {
            locked
                .get_event(Some(&event_id.to_string()), None)?
                .ok_or_else(|| anyhow::anyhow!("event '{event_id}' does not exist"))?
                .event
                .correlation_id
        } else if let Some(correlation_id) = query.correlation_id.as_deref() {
            Uuid::parse_str(correlation_id)?
        } else {
            anyhow::bail!("event_id or correlation_id is required");
        };
        let mut events = locked.query_events(&StoreEventQuery {
            correlation_id: Some(correlation),
            limit: 1_001,
            ..StoreEventQuery::default()
        })?;
        let events_truncated = events.len() > 1_000;
        if events_truncated {
            events.remove(0);
        }
        if events.is_empty() {
            anyhow::bail!("correlation '{correlation}' has no events");
        }
        let mut logs = locked.query_logs(&StoreLogQuery {
            correlation_id: Some(correlation),
            limit: 2_001,
            ..StoreLogQuery::default()
        })?;
        let logs_truncated = logs.len() > 2_000;
        if logs_truncated {
            logs.remove(0);
        }
        Ok(build_trace_response(
            focus,
            correlation,
            events,
            logs,
            events_truncated || logs_truncated,
        ))
    })();
    match result {
        Ok(trace) => json_response(StatusCode::OK, trace),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

fn build_trace_response(
    focus: Option<Uuid>,
    correlation: Uuid,
    events: Vec<crate::event::StoredEvent>,
    logs: Vec<crate::event::StoredLog>,
    truncated: bool,
) -> Value {
    let parents = events
        .iter()
        .map(|stored| (stored.event.id, stored.event.causation_id))
        .collect::<HashMap<_, _>>();
    let roots = parents
        .keys()
        .map(|id| event_root(*id, &parents))
        .collect::<BTreeSet<_>>();
    let mut children = HashMap::<Uuid, Vec<Uuid>>::new();
    for stored in &events {
        if let Some(parent) = stored.event.causation_id {
            children.entry(parent).or_default().push(stored.event.id);
        }
    }
    let enriched_events = events
        .into_iter()
        .map(|stored| {
            let id = stored.event.id;
            json!({
                "record": stored,
                "root_event_id": event_root(id, &parents),
                "caused_event_ids": children.remove(&id).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    let enriched_logs = logs
        .into_iter()
        .map(|stored| {
            let root = stored
                .log
                .event_id
                .filter(|id| parents.contains_key(id))
                .map(|id| event_root(id, &parents));
            json!({ "record": stored, "root_event_id": root })
        })
        .collect::<Vec<_>>();
    json!({
        "focus_event_id": focus,
        "correlation_id": correlation,
        "root_event_ids": roots,
        "truncated": truncated,
        "events": enriched_events,
        "logs": enriched_logs,
    })
}

fn event_root(event_id: Uuid, parents: &HashMap<Uuid, Option<Uuid>>) -> Uuid {
    let mut current = event_id;
    let mut visited = HashSet::new();
    while visited.insert(current) {
        let Some(Some(parent)) = parents.get(&current) else {
            break;
        };
        if !parents.contains_key(parent) {
            break;
        }
        current = *parent;
    }
    current
}

fn build_event_query(query: EventsQuery) -> anyhow::Result<StoreEventQuery> {
    let occurred_after = if let Some(from) = query.from {
        Some(DateTime::parse_from_rfc3339(&from)?.with_timezone(&Utc))
    } else {
        match query.window.as_deref().unwrap_or("all") {
            "all" => None,
            "15m" => Some(Utc::now() - Duration::minutes(15)),
            "1h" => Some(Utc::now() - Duration::hours(1)),
            "24h" => Some(Utc::now() - Duration::hours(24)),
            "7d" => Some(Utc::now() - Duration::days(7)),
            "30d" => Some(Utc::now() - Duration::days(30)),
            value => anyhow::bail!("unsupported time window '{value}'"),
        }
    };
    let occurred_before = query
        .to
        .map(|value| DateTime::parse_from_rfc3339(&value).map(|value| value.with_timezone(&Utc)))
        .transpose()?;
    let event_id = query
        .event_id
        .map(|value| Uuid::parse_str(&value))
        .transpose()?;
    let causation_id = query
        .causation_id
        .map(|value| Uuid::parse_str(&value))
        .transpose()?;
    let correlation_id = query
        .correlation_id
        .map(|value| Uuid::parse_str(&value))
        .transpose()?;

    Ok(StoreEventQuery {
        event_type: query.event_type.filter(|value| !value.is_empty()),
        event_type_prefix: query.prefix.filter(|value| !value.is_empty()),
        source: query.source.filter(|value| !value.is_empty()),
        event_id,
        causation_id,
        correlation_id,
        before_sequence: query.before_sequence,
        after_sequence: query.after_sequence,
        occurred_after,
        occurred_before,
        payload_contains: query.payload_contains.filter(|value| !value.is_empty()),
        limit: query.limit.unwrap_or(100).clamp(1, 1_000),
    })
}

async fn models(State(state): State<WebState>) -> Response {
    match state.engine.model_catalog() {
        Ok(catalog) => json_response(
            StatusCode::OK,
            json!({
                "active": {
                    "provider": state.engine.model_provider(),
                    "model": state.engine.model_name(),
                },
                "catalog": catalog
            }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn refresh_models(State(state): State<WebState>) -> Response {
    match state.engine.refresh_model_catalog().await {
        Ok(catalog) => json_response(StatusCode::OK, json!({ "catalog": catalog })),
        Err(error) => json_response(
            StatusCode::BAD_GATEWAY,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn stats(State(state): State<WebState>) -> Response {
    match state
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("event store lock poisoned"))
        .and_then(|store| store.usage_stats())
    {
        Ok(stats) => json_response(
            StatusCode::OK,
            json!({ "generated_at": Utc::now(), "usage": stats }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    level: Option<String>,
    category: Option<String>,
    name: Option<String>,
    name_prefix: Option<String>,
    dispatch_id: Option<String>,
    event_id: Option<String>,
    correlation_id: Option<String>,
    action_group_id: Option<String>,
    action_id: Option<String>,
    tool_call_id: Option<String>,
    before_sequence: Option<i64>,
    after_sequence: Option<i64>,
    from: Option<String>,
    to: Option<String>,
    window: Option<String>,
    payload_contains: Option<String>,
    limit: Option<usize>,
}

async fn list_logs(State(state): State<WebState>, Query(query): Query<LogsQuery>) -> Response {
    match build_log_query(query).and_then(|query| {
        state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .query_logs(&query)
    }) {
        Ok(logs) => json_response(StatusCode::OK, json!({ "logs": logs })),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
    }
}

fn build_log_query(query: LogsQuery) -> anyhow::Result<StoreLogQuery> {
    let occurred_after = if let Some(from) = query.from {
        Some(DateTime::parse_from_rfc3339(&from)?.with_timezone(&Utc))
    } else {
        window_start(query.window.as_deref().unwrap_or("all"))?
    };
    let occurred_before = query
        .to
        .map(|value| DateTime::parse_from_rfc3339(&value).map(|value| value.with_timezone(&Utc)))
        .transpose()?;
    let uuid = |value: Option<String>| -> anyhow::Result<Option<Uuid>> {
        value
            .filter(|value| !value.is_empty())
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(Into::into)
    };
    Ok(StoreLogQuery {
        level: query.level.filter(|value| !value.is_empty()),
        category: query.category.filter(|value| !value.is_empty()),
        name: query.name.filter(|value| !value.is_empty()),
        name_prefix: query.name_prefix.filter(|value| !value.is_empty()),
        dispatch_id: uuid(query.dispatch_id)?,
        event_id: uuid(query.event_id)?,
        correlation_id: uuid(query.correlation_id)?,
        action_group_id: query.action_group_id.filter(|value| !value.is_empty()),
        action_id: query.action_id.filter(|value| !value.is_empty()),
        tool_call_id: query.tool_call_id.filter(|value| !value.is_empty()),
        before_sequence: query.before_sequence,
        after_sequence: query.after_sequence,
        occurred_after,
        occurred_before,
        payload_contains: query.payload_contains.filter(|value| !value.is_empty()),
        limit: query.limit.unwrap_or(100).clamp(1, 1000),
    })
}

fn window_start(window: &str) -> anyhow::Result<Option<DateTime<Utc>>> {
    Ok(match window {
        "all" => None,
        "15m" => Some(Utc::now() - Duration::minutes(15)),
        "1h" => Some(Utc::now() - Duration::hours(1)),
        "24h" => Some(Utc::now() - Duration::hours(24)),
        "7d" => Some(Utc::now() - Duration::days(7)),
        "30d" => Some(Utc::now() - Duration::days(30)),
        value => anyhow::bail!("unsupported time window '{value}'"),
    })
}

async fn list_extensions(State(state): State<WebState>) -> impl IntoResponse {
    axum::Json(state.extensions.summaries())
}

#[derive(Deserialize)]
struct ExtensionToggle {
    enabled: bool,
}

async fn toggle_extension(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
    axum::Json(toggle): axum::Json<ExtensionToggle>,
) -> Response {
    let _catalog_mutation_guard = state.catalog_mutation_lock.lock().await;
    match state.extensions.set_enabled(&extension_id, toggle.enabled) {
        Ok(true) => json_response(
            StatusCode::OK,
            json!({ "id": extension_id, "enabled": toggle.enabled }),
        ),
        Ok(false) => json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "extension not found" }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

fn require_local_studio(state: &WebState) -> Option<Response> {
    (!state.local_admin).then(|| {
        json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Extension Studio is available only on a loopback bind" }),
        )
    })
}

async fn list_drafts(State(state): State<WebState>) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || studio.list_drafts()).await {
        Ok(Ok(drafts)) => json_response(StatusCode::OK, json!(drafts)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn create_draft(
    State(state): State<WebState>,
    axum::Json(request): axum::Json<CreateDraftRequest>,
) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || studio.create_draft(request)).await {
        Ok(Ok(draft)) => json_response(StatusCode::CREATED, json!(draft)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn list_draft_files(State(state): State<WebState>, Path(draft_id): Path<String>) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || studio.list_files(&draft_id)).await {
        Ok(Ok(files)) => json_response(StatusCode::OK, json!({ "files": files })),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn read_draft_file(
    State(state): State<WebState>,
    Path((draft_id, path)): Path<(String, String)>,
) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || studio.read_file(DraftFileRequest { draft_id, path }))
        .await
    {
        Ok(Ok(file)) => json_response(StatusCode::OK, json!(file)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Deserialize)]
struct DraftWriteBody {
    content: String,
    expected_sha256: Option<String>,
}

async fn write_draft_file(
    State(state): State<WebState>,
    Path((draft_id, path)): Path<(String, String)>,
    axum::Json(body): axum::Json<DraftWriteBody>,
) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || {
        studio.write_file(WriteDraftFileRequest {
            draft_id,
            path,
            content: body.content,
            expected_sha256: body.expected_sha256,
        })
    })
    .await
    {
        Ok(Ok(file)) => json_response(StatusCode::OK, json!(file)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Deserialize)]
struct DraftDirectoryBody {
    path: String,
}

async fn create_draft_directory(
    State(state): State<WebState>,
    Path(draft_id): Path<String>,
    axum::Json(body): axum::Json<DraftDirectoryBody>,
) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || {
        studio.create_directory(CreateDraftDirectoryRequest {
            draft_id,
            path: body.path,
        })
    })
    .await
    {
        Ok(Ok(())) => json_response(StatusCode::CREATED, json!({ "created": true })),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn validate_draft(State(state): State<WebState>, Path(draft_id): Path<String>) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let studio = state.studio.clone();
    match tokio::task::spawn_blocking(move || studio.validate(&draft_id)).await {
        Ok(Ok(validation)) => json_response(StatusCode::OK, json!(validation)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

#[derive(Deserialize)]
struct DraftInstallBody {
    approved_hash: String,
}

async fn install_draft(
    State(state): State<WebState>,
    Path(draft_id): Path<String>,
    axum::Json(body): axum::Json<DraftInstallBody>,
) -> Response {
    if let Some(response) = require_local_studio(&state) {
        return response;
    }
    let _catalog_mutation_guard = state.catalog_mutation_lock.lock().await;
    let draft_path = match state.studio.draft_path(&draft_id) {
        Ok(path) => path,
        Err(error) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            );
        }
    };
    let existed = state.extensions.get(&draft_id).is_some();
    let extensions_dir = state.extensions_dir.clone();
    let approved_hash = body.approved_hash;
    let installed = match tokio::task::spawn_blocking(move || {
        ExtensionInstaller::new(extensions_dir).install_local_if_hash(&draft_path, &approved_hash)
    })
    .await
    {
        Ok(Ok(installed)) => installed,
        Ok(Err(error)) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            );
        }
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": error.to_string() }),
            );
        }
    };
    match state.extensions.reload(&installed.id) {
        Ok(_) => json_response(StatusCode::OK, json!({ "installed": installed })),
        Err(error) => {
            let extensions_dir = state.extensions_dir.clone();
            let extension_id = installed.id.clone();
            let recovery = tokio::task::spawn_blocking(move || {
                let installer = ExtensionInstaller::new(extensions_dir);
                if existed {
                    installer.rollback(&extension_id).map(|_| ())
                } else {
                    installer.remove_installed(&extension_id)
                }
            })
            .await;
            let restored = matches!(recovery, Ok(Ok(())))
                && (!existed || state.extensions.reload(&installed.id).is_ok());
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": format!("installed draft could not be loaded: {error}"),
                    "rolled_back": restored
                }),
            )
        }
    }
}

#[derive(Deserialize)]
struct ExtensionGrantsUpdate {
    #[serde(default)]
    filesystem_roots: Vec<String>,
    #[serde(default)]
    process_executables: BTreeMap<String, String>,
}

async fn extension_grants(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
) -> Response {
    let Some(extension) = state.extensions.get(&extension_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "extension not found" }),
        );
    };
    if !extension.manifest.capabilities.filesystem && !extension.manifest.capabilities.process {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "extension does not request managed grants" }),
        );
    }
    match state
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("event store lock poisoned"))
    {
        Ok(store) => {
            let filesystem_roots = store.extension_filesystem_roots(&extension_id);
            let process_executables = store.extension_process_executables(&extension_id);
            match (filesystem_roots, process_executables) {
                (Ok(filesystem_roots), Ok(process_executables)) => json_response(
                    StatusCode::OK,
                    json!({
                        "filesystem_roots": filesystem_roots,
                        "process_executables": process_executables
                    }),
                ),
                (Err(error), _) | (_, Err(error)) => json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": error.to_string() }),
                ),
            }
        }
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_extension_grants(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
    axum::Json(update): axum::Json<ExtensionGrantsUpdate>,
) -> Response {
    let _catalog_mutation_guard = state.catalog_mutation_lock.lock().await;
    let Some(extension) = state.extensions.get(&extension_id) else {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "extension not found" }),
        );
    };
    if !extension.manifest.capabilities.filesystem && !extension.manifest.capabilities.process {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "extension does not request managed grants" }),
        );
    }
    let filesystem_enabled = extension.manifest.capabilities.filesystem;
    let process_enabled = extension.manifest.capabilities.process;
    #[cfg(not(target_os = "linux"))]
    if process_enabled {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "process capability is supported only on Linux" }),
        );
    }
    let normalized = match tokio::task::spawn_blocking(move || {
        let roots = if filesystem_enabled {
            normalize_grant_roots(&update.filesystem_roots)?
        } else {
            Vec::new()
        };
        #[cfg(target_os = "linux")]
        let executables = if process_enabled {
            normalize_executable_grants(&update.process_executables)?
        } else {
            Vec::new()
        };
        #[cfg(not(target_os = "linux"))]
        let executables = Vec::new();
        Ok::<_, anyhow::Error>((roots, executables))
    })
    .await
    {
        Ok(Ok(roots)) => roots,
        Ok(Err(error)) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            );
        }
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": error.to_string() }),
            );
        }
    };
    let (filesystem_roots, process_executables) = normalized;
    match state
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("event store lock poisoned"))
        .and_then(|mut store| {
            if filesystem_enabled {
                store.set_extension_filesystem_roots(&extension_id, &filesystem_roots)?;
            }
            if process_enabled {
                store.set_extension_process_executables(&extension_id, &process_executables)?;
            }
            Ok(())
        }) {
        Ok(()) => json_response(
            StatusCode::OK,
            json!({
                "filesystem_roots": filesystem_roots,
                "process_executables": process_executables
            }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn check_extension_update(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
) -> Response {
    let extensions_dir = state.extensions_dir.clone();
    match tokio::task::spawn_blocking(move || {
        ExtensionInstaller::new(extensions_dir).check_update(&extension_id)
    })
    .await
    {
        Ok(Ok(status)) => json_response(StatusCode::OK, json!(status)),
        Ok(Err(error)) => json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": error.to_string() }),
        ),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn update_extension(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
) -> Response {
    let _catalog_mutation_guard = state.catalog_mutation_lock.lock().await;
    let extensions_dir = state.extensions_dir.clone();
    let rollback_dir = extensions_dir.clone();
    let update_id = extension_id.clone();
    let installed = match tokio::task::spawn_blocking(move || {
        ExtensionInstaller::new(extensions_dir).update(&update_id)
    })
    .await
    {
        Ok(Ok(installed)) => installed,
        Ok(Err(error)) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string() }),
            );
        }
        Err(error) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": error.to_string() }),
            );
        }
    };
    match state.extensions.reload(&extension_id) {
        Ok(_) => json_response(StatusCode::OK, json!({ "installed": installed })),
        Err(error) => {
            let rollback_id = extension_id.clone();
            let rollback = tokio::task::spawn_blocking(move || {
                ExtensionInstaller::new(rollback_dir).rollback(&rollback_id)
            })
            .await;
            let restored =
                matches!(rollback, Ok(Ok(_))) && state.extensions.reload(&extension_id).is_ok();
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": format!("extension update could not be loaded: {error}"),
                    "rolled_back": restored
                }),
            )
        }
    }
}

async fn reload_extension(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
) -> Response {
    let _catalog_mutation_guard = state.catalog_mutation_lock.lock().await;
    match state.extensions.reload(&extension_id) {
        Ok(extension) => json_response(
            StatusCode::OK,
            json!({ "id": extension.manifest.id, "version": extension.manifest.version }),
        ),
        Err(error) => {
            let rollback_dir = state.extensions_dir.clone();
            let rollback_id = extension_id.clone();
            let rollback = tokio::task::spawn_blocking(move || {
                ExtensionInstaller::new(rollback_dir).rollback(&rollback_id)
            })
            .await;
            let restored =
                matches!(rollback, Ok(Ok(_))) && state.extensions.reload(&extension_id).is_ok();
            json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": error.to_string(), "rolled_back": restored }),
            )
        }
    }
}

async fn extension_root(
    State(state): State<WebState>,
    Path(extension_id): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle_extension_request(state, extension_id, "/".into(), method, uri, headers, body).await
}

async fn extension_path(
    State(state): State<WebState>,
    Path((extension_id, path)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle_extension_request(
        state,
        extension_id,
        format!("/{path}"),
        method,
        uri,
        headers,
        body,
    )
    .await
}

async fn handle_extension_request(
    state: WebState,
    extension_id: String,
    path: String,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match handle_extension_request_inner(state, extension_id, path, method, uri, headers, body)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            eprintln!("web request failed: {error:#}");
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": error.to_string() }),
            )
        }
    }
}

async fn handle_extension_request_inner(
    state: WebState,
    extension_id: String,
    path: String,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> anyhow::Result<Response> {
    let Some(extension) = state.extensions.get(&extension_id) else {
        return Ok(json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "extension not found" }),
        ));
    };
    if !extension.is_enabled() {
        return Ok(json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "extension is disabled", "extension_id": extension_id }),
        ));
    }

    let body = String::from_utf8(body.to_vec()).context("request body is not UTF-8")?;
    let expects_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    let parsed_json = if body.trim().is_empty() {
        None
    } else if expects_json {
        match serde_json::from_str(&body) {
            Ok(value) => Some(value),
            Err(error) => {
                return Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": format!("invalid JSON: {error}") }),
                ));
            }
        }
    } else {
        None
    };
    let request = RequestData {
        method: method.to_string(),
        path: path.clone(),
        path_params: HashMap::new(),
        query: uri
            .query()
            .map(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_default(),
        headers: headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect(),
        json: parsed_json,
        body,
    };

    if let Some(mut outcome) =
        tokio::task::block_in_place(|| extension.handle_route(method.as_str(), &path, request))?
    {
        process_route_outcome(&state, &extension_id, &mut outcome).await?;
        return route_response(outcome);
    }

    if method == Method::GET
        && let Some((contents, content_type)) = extension.static_file(&path)?
    {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(contents))
            .context("failed to build static response");
    }

    Ok(json_response(
        StatusCode::NOT_FOUND,
        json!({ "error": "route not found" }),
    ))
}

async fn process_route_outcome(
    state: &WebState,
    extension_id: &str,
    outcome: &mut RouteOutcome,
) -> anyhow::Result<()> {
    let Some(draft) = outcome.emit.take() else {
        return Ok(());
    };

    validate_event_namespace(extension_id, &draft.event_type)?;
    let correlation_id = Uuid::now_v7();
    let event = Event::new(
        draft.event_type,
        format!("extension:{extension_id}"),
        correlation_id,
        None,
        draft.payload,
    );
    let (sequence, accepted_event, accepted_json, _reused) = {
        let store = state
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
        if let Some(key) = draft.idempotency_key.as_deref() {
            anyhow::ensure!(
                outcome.json.is_some(),
                "idempotent event routes require a JSON response"
            );
            store.append_and_enqueue_idempotent(&event, key, outcome.json.as_ref())?
        } else {
            let sequence = store.append_and_enqueue(&event)?;
            (sequence, event, outcome.json.clone(), false)
        }
    };
    outcome.json = accepted_json;
    outcome.status = StatusCode::ACCEPTED.as_u16();
    add_response_field(
        outcome,
        "event_id",
        Value::String(accepted_event.id.to_string()),
    );
    add_response_field(
        outcome,
        "correlation_id",
        Value::String(accepted_event.correlation_id.to_string()),
    );
    add_response_field(outcome, "sequence", Value::Number(sequence.into()));
    Ok(())
}

fn validate_event_namespace(extension_id: &str, event_type: &str) -> anyhow::Result<()> {
    if !event_type.starts_with(&format!("{extension_id}.")) {
        anyhow::bail!("extension '{extension_id}' cannot emit event type '{event_type}'");
    }
    Ok(())
}

fn add_response_field(outcome: &mut RouteOutcome, key: &str, value: Value) {
    let json = outcome
        .json
        .get_or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(object) = json {
        object.insert(key.to_owned(), value);
    }
}

fn route_response(outcome: RouteOutcome) -> anyhow::Result<Response> {
    let status =
        StatusCode::from_u16(outcome.status).context("extension returned invalid HTTP status")?;
    if let Some(json) = outcome.json {
        return Ok(json_response(status, json));
    }
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            outcome
                .content_type
                .unwrap_or_else(|| "text/plain; charset=utf-8".into()),
        )
        .body(Body::from(outcome.body.unwrap_or_default()))
        .context("failed to build extension response")
}

fn html_response(html: &'static str) -> Response {
    asset_response("text/html; charset=utf-8", html.as_bytes())
}

fn asset_response(content_type: &'static str, contents: &'static [u8]) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(contents))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn json_response(status: StatusCode, value: Value) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&value).unwrap_or_default()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::StoredEvent;

    #[test]
    fn event_graph_query_preserves_filters_and_clamps_the_sentinel_limit() {
        let correlation = Uuid::now_v7();
        let (query, limit) = build_event_graph_query(EventGraphQuery {
            event_type: Some("chat.message.created".into()),
            source: Some("extension:chat".into()),
            correlation_id: Some(correlation.to_string()),
            limit: Some(usize::MAX),
        })
        .unwrap();
        assert_eq!(limit, 1_000);
        assert_eq!(query.limit, 1_001);
        assert_eq!(query.event_type.as_deref(), Some("chat.message.created"));
        assert_eq!(query.source.as_deref(), Some("extension:chat"));
        assert_eq!(query.correlation_id, Some(correlation));

        let (minimum, limit) = build_event_graph_query(EventGraphQuery {
            event_type: None,
            source: None,
            correlation_id: None,
            limit: Some(0),
        })
        .unwrap();
        assert_eq!(limit, 1);
        assert_eq!(minimum.limit, 2);
        assert!(
            build_event_graph_query(EventGraphQuery {
                event_type: None,
                source: None,
                correlation_id: Some("not-a-uuid".into()),
                limit: None,
            })
            .is_err()
        );
    }

    #[test]
    fn event_graph_cursor_is_global_when_filters_are_old_or_empty() {
        let store = EventStore::open(":memory:").unwrap();
        let correlation = Uuid::now_v7();
        store
            .append(&Event::new(
                "old,type",
                "wanted",
                correlation,
                None,
                json!({}),
            ))
            .unwrap();
        store
            .append(&Event::new(
                "new.type",
                "other",
                Uuid::now_v7(),
                None,
                json!({}),
            ))
            .unwrap();

        let old = query_event_graph(
            &store,
            &StoreEventQuery {
                source: Some("wanted".into()),
                limit: 11,
                ..StoreEventQuery::default()
            },
            10,
        )
        .unwrap();
        assert_eq!(old["events"][0]["sequence"], 1);
        assert_eq!(old["cursor"], 2);

        let empty = query_event_graph(
            &store,
            &StoreEventQuery {
                source: Some("missing".into()),
                limit: 11,
                ..StoreEventQuery::default()
            },
            10,
        )
        .unwrap();
        assert_eq!(empty["events"], json!([]));
        assert_eq!(empty["cursor"], 2);
    }

    #[test]
    fn event_graph_preserves_event_relationship_facts_and_bounds() {
        let correlation = Uuid::now_v7();
        let root = Event::new("test.root", "test", correlation, None, json!({}));
        let child = Event::new("test.child", "test", correlation, Some(root.id), json!({}));
        let response = build_event_graph_response(
            vec![
                StoredEvent {
                    sequence: 1,
                    event: root.clone(),
                },
                StoredEvent {
                    sequence: 2,
                    event: child.clone(),
                },
            ],
            vec![json!({
                "link_id": "link-1",
                "from_event_id": root.id,
                "to_event_id": child.id,
                "relation": "supports",
                "bidirectional": false,
            })],
            true,
            false,
            99,
        );
        assert_eq!(response["events"][1]["causation_id"], json!(root.id));
        assert_eq!(response["events"][1]["correlation_id"], json!(correlation));
        assert_eq!(response["links"][0]["relation"], "supports");
        assert_eq!(response["events_truncated"], true);
        assert_eq!(response["links_truncated"], false);
        assert_eq!(response["cursor"], 99);
    }

    #[test]
    fn trace_reports_each_events_causal_root_and_children() {
        let correlation = Uuid::now_v7();
        let root = Event::new("test.root", "test", correlation, None, json!({}));
        let child = Event::new("test.child", "test", correlation, Some(root.id), json!({}));
        let response = build_trace_response(
            Some(child.id),
            correlation,
            vec![
                StoredEvent {
                    sequence: 1,
                    event: root.clone(),
                },
                StoredEvent {
                    sequence: 2,
                    event: child.clone(),
                },
            ],
            Vec::new(),
            false,
        );
        assert_eq!(response["root_event_ids"], json!([root.id]));
        assert_eq!(response["events"][1]["root_event_id"], json!(root.id));
        assert_eq!(response["events"][0]["caused_event_ids"], json!([child.id]));
        assert_eq!(response["truncated"], false);
    }
}

#[cfg(test)]
mod sse_tests {
    use super::*;

    #[test]
    fn sse_cursor_precedence_and_validation_are_explicit() {
        let header = axum::http::HeaderValue::from_static("8");
        assert_eq!(
            parse_stream_cursor(Some("9"), Some(&header), 10).unwrap(),
            8
        );
        assert_eq!(parse_stream_cursor(None, Some(&header), 10).unwrap(), 8);
        assert_eq!(parse_stream_cursor(None, None, 10).unwrap(), 10);
        assert!(parse_stream_cursor(Some("-1"), None, 10).is_err());
        assert!(parse_stream_cursor(Some("nope"), None, 10).is_err());
    }

    #[test]
    fn sse_type_contract_is_comma_separated_exact_values() {
        assert_eq!(
            parse_stream_event_types(Some("chat.one, chat.two")).unwrap(),
            vec!["chat.one", "chat.two"]
        );
        assert!(parse_stream_event_types(Some(" , ")).is_err());
        assert_eq!(
            parse_stream_exact_type(Some("chat,comma"))
                .unwrap()
                .as_deref(),
            Some("chat,comma")
        );
        assert!(parse_stream_exact_type(Some("")).is_err());
    }

    #[test]
    fn sse_cursor_frame_preserves_live_only_anchor_before_data() {
        assert_eq!(
            sse_cursor_frame(41),
            "id: 41\nevent: habibi.cursor\ndata: {\"sequence\":41}\n\n"
        );
    }

    #[test]
    fn sse_frame_has_sequence_id_named_event_and_full_json_data() {
        assert_eq!(
            sse_event_frame(42, r#"{"sequence":42}"#),
            "id: 42\nevent: habibi.event\ndata: {\"sequence\":42}\n\n"
        );
    }
}
