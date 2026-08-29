use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
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
    event::Event,
    extension::{ExtensionManager, RequestData, RouteOutcome},
    filesystem::normalize_grant_roots,
    installer::ExtensionInstaller,
    reactor::Reactor,
    store::{SharedEventStore, StoreEventQuery, StoreLogQuery},
    studio::{
        CreateDraftDirectoryRequest, CreateDraftRequest, DraftFileRequest, StudioService,
        WriteDraftFileRequest,
    },
};

#[derive(Clone)]
pub struct WebState {
    pub extensions: Arc<ExtensionManager>,
    pub reactor: Arc<Reactor>,
    pub store: SharedEventStore,
    pub extensions_dir: PathBuf,
    pub studio: Arc<StudioService>,
    pub local_admin: bool,
    pub reaction_lock: Arc<tokio::sync::Mutex<()>>,
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
        .route("/assets/markdown.js", get(markdown_js_asset))
        .route("/assets/stats.js", get(stats_js_asset))
        .route("/assets/studio.js", get(studio_js_asset))
        .route("/api/events", get(list_events))
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
                .trigger_event_id
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
    match state.reactor.model_catalog() {
        Ok(catalog) => json_response(StatusCode::OK, json!({ "catalog": catalog })),
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": error.to_string() }),
        ),
    }
}

async fn refresh_models(State(state): State<WebState>) -> Response {
    match state.reactor.refresh_model_catalog().await {
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
    reaction_id: Option<String>,
    trigger_event_id: Option<String>,
    correlation_id: Option<String>,
    batch_id: Option<String>,
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
        reaction_id: uuid(query.reaction_id)?,
        trigger_event_id: uuid(query.trigger_event_id)?,
        correlation_id: uuid(query.correlation_id)?,
        batch_id: query.batch_id.filter(|value| !value.is_empty()),
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
    let _reaction_guard = state.reaction_lock.lock().await;
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
    let _reaction_guard = state.reaction_lock.lock().await;
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
    let _reaction_guard = state.reaction_lock.lock().await;
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
    let _reaction_guard = state.reaction_lock.lock().await;
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
    let _reaction_guard = state.reaction_lock.lock().await;
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

    let _reaction_guard = state.reaction_lock.lock().await;
    validate_event_namespace(extension_id, &draft.event_type)?;
    let correlation_id = Uuid::now_v7();
    let trigger = Event::new(
        draft.event_type,
        format!("extension:{extension_id}"),
        correlation_id,
        None,
        draft.payload,
    );
    append(&state.store, &trigger)?;
    add_response_field(outcome, "event_id", Value::String(trigger.id.to_string()));

    let reaction_result = state.reactor.react(&trigger).await;

    match reaction_result {
        Ok(()) => {
            add_response_field(
                outcome,
                "correlation_id",
                Value::String(correlation_id.to_string()),
            );
        }
        Err(error) => {
            outcome.status = StatusCode::ACCEPTED.as_u16();
            add_response_field(outcome, "reaction_error", Value::String(error.to_string()));
        }
    }
    Ok(())
}

fn validate_event_namespace(extension_id: &str, event_type: &str) -> anyhow::Result<()> {
    if !event_type.starts_with(&format!("{extension_id}.")) {
        anyhow::bail!("extension '{extension_id}' cannot emit event type '{event_type}'");
    }
    Ok(())
}

fn append(store: &SharedEventStore, event: &Event) -> anyhow::Result<i64> {
    store
        .lock()
        .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
        .append(event)
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
