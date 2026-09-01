use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use anyhow::{Context, Result, bail};
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, RegistryKey, StdLib, Value as LuaValue,
    VmState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use crate::process::{ProcessHost, ProcessRequest};

use crate::{
    context::ContextContribution,
    embedding::EventEmbeddingIndex,
    event::Event,
    filesystem::{
        FilesystemHost, MoveRequest, PatchRequest, PathRequest, SearchRequest, WriteRequest,
    },
    installer::{ExtensionInstaller, InstallMetadata},
    search::{SearchHost, SearchRequest as WebSearchRequest},
    store::{SharedEventStore, StoreEventQuery},
    studio::{
        CreateDraftDirectoryRequest, CreateDraftRequest, DraftFileRequest, StudioHost,
        WriteDraftFileRequest,
    },
    tool::{HostEffect, ToolCall, ToolContext, ToolDefinition, ToolExecution, provider_tool_name},
};

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub api_version: u32,
    #[serde(default)]
    pub capabilities: ExtensionCapabilities,
    #[serde(default)]
    pub web: Option<ExtensionWebConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ExtensionCapabilities {
    #[serde(default)]
    pub web: bool,
    #[serde(default)]
    pub kv: bool,
    #[serde(default)]
    pub events: bool,
    #[serde(default)]
    pub tools: bool,
    #[serde(default)]
    pub context: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub process: bool,
    #[serde(default)]
    pub studio: bool,
    #[serde(default)]
    pub search: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionWebConfig {
    pub static_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestData {
    pub method: String,
    pub path: String,
    pub path_params: HashMap<String, String>,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub json: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteOutcome {
    #[serde(default = "default_status")]
    pub status: u16,
    pub json: Option<Value>,
    pub body: Option<String>,
    pub content_type: Option<String>,
    pub emit: Option<EventDraft>,
}

fn default_status() -> u16 {
    200
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventDraft {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    #[serde(rename = "type")]
    event_type: Option<String>,
    prefix: Option<String>,
    before_sequence: Option<i64>,
    after_sequence: Option<i64>,
    #[serde(default = "default_query_limit")]
    limit: usize,
}

fn default_query_limit() -> usize {
    100
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticEventQuery {
    text: String,
    before_sequence: i64,
    #[serde(default = "default_semantic_event_limit")]
    limit: usize,
    #[serde(default = "default_semantic_event_similarity")]
    minimum_similarity: f32,
}

fn default_semantic_event_limit() -> usize {
    crate::embedding::SEMANTIC_EVENT_LIMIT
}

fn default_semantic_event_similarity() -> f32 {
    crate::embedding::MIN_EVENT_SIMILARITY
}

struct RegisteredTool {
    definition: ToolDefinition,
    handler: RegistryKey,
}

struct RegisteredHook {
    name: String,
    handler: RegistryKey,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextHookExecution {
    pub extension_id: String,
    pub hook: String,
    pub duration_ms: u128,
    pub contribution: Option<ContextContribution>,
    pub error: Option<String>,
}

type SharedEventEmbeddingIndex = Arc<RwLock<Option<Arc<EventEmbeddingIndex>>>>;

struct RegisteredRoute {
    method: String,
    path: String,
    handler: RegistryKey,
}

struct LuaState {
    lua: Lua,
    instruction_budget: Arc<AtomicU64>,
    routes: Vec<RegisteredRoute>,
    tools: Vec<RegisteredTool>,
    context_hooks: Vec<RegisteredHook>,
    filesystem_host: Option<FilesystemHost>,
    #[cfg(target_os = "linux")]
    process_host: Option<ProcessHost>,
    studio_host: Option<StudioHost>,
    search_host: Option<SearchHost>,
}

pub struct LoadedExtension {
    pub manifest: ExtensionManifest,
    pub generation: String,
    execution_snapshot: tempfile::TempDir,
    static_files: HashMap<String, (Vec<u8>, String)>,
    store: SharedEventStore,
    enabled: AtomicBool,
    state: Mutex<LuaState>,
}

impl LoadedExtension {
    pub(crate) fn load(directory: &Path, store: SharedEventStore) -> Result<Self> {
        Self::load_with_embeddings(directory, store, Arc::new(RwLock::new(None)))
    }

    fn load_with_embeddings(
        directory: &Path,
        store: SharedEventStore,
        event_embeddings: SharedEventEmbeddingIndex,
    ) -> Result<Self> {
        let manifest_path = directory.join("extension.toml");
        let manifest_source = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: ExtensionManifest = toml::from_str(&manifest_source)
            .with_context(|| format!("invalid extension manifest {}", manifest_path.display()))?;
        validate_manifest(&manifest)?;
        let enabled = store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .extension_enabled(&manifest.id)?;

        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(32 * 1024 * 1024)?;
        let instruction_budget = Arc::new(AtomicU64::new(100));
        let hook_budget = instruction_budget.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(10_000),
            move |_, _| {
                if hook_budget
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                        remaining.checked_sub(1)
                    })
                    .is_err()
                {
                    return Err(mlua::Error::runtime("extension instruction limit exceeded"));
                }
                Ok(VmState::Continue)
            },
        )?;
        for global in ["dofile", "loadfile", "require"] {
            lua.globals().raw_remove(global)?;
        }
        let registered_routes = Arc::new(Mutex::new(Vec::new()));
        let registered_tools = Arc::new(Mutex::new(Vec::new()));
        let context_hooks = Arc::new(Mutex::new(Vec::new()));
        let filesystem_host = manifest
            .capabilities
            .filesystem
            .then(|| FilesystemHost::new(&manifest.id, store.clone()));
        #[cfg(target_os = "linux")]
        let process_host = manifest
            .capabilities
            .process
            .then(|| ProcessHost::new(&manifest.id, store.clone()));
        #[cfg(not(target_os = "linux"))]
        if manifest.capabilities.process {
            bail!("process capability is supported only on Linux");
        }
        let studio_host = manifest
            .capabilities
            .studio
            .then(StudioHost::from_env)
            .transpose()?;
        let search_host = manifest
            .capabilities
            .search
            .then(SearchHost::from_env)
            .transpose()?;
        let habibi = lua.create_table()?;
        habibi.set(
            "id",
            lua.create_function(|_, ()| Ok(uuid::Uuid::now_v7().to_string()))?,
        )?;
        habibi.set(
            "array",
            lua.create_function(|lua, table: mlua::Table| {
                table.set_metatable(Some(lua.array_metatable()))?;
                Ok(table)
            })?,
        )?;
        let json_api = lua.create_table()?;
        json_api.set(
            "encode",
            lua.create_function(|lua, value: LuaValue| {
                let value: Value = lua.from_value(value)?;
                serde_json::to_string(&value).map_err(mlua::Error::external)
            })?,
        )?;
        habibi.set("json", json_api)?;

        if manifest.capabilities.kv {
            habibi.set("kv", create_kv_api(&lua, &manifest.id, store.clone())?)?;
        }
        if manifest.capabilities.events {
            habibi.set(
                "events",
                create_events_api(&lua, store.clone(), event_embeddings)?,
            )?;
        }
        if let Some(host) = &filesystem_host {
            habibi.set("files", create_files_api(&lua, host.clone())?)?;
        }
        #[cfg(target_os = "linux")]
        if let Some(host) = &process_host {
            habibi.set("process", create_process_api(&lua, host.clone())?)?;
        }
        if let Some(host) = &studio_host {
            habibi.set("studio", create_studio_api(&lua, host.clone())?)?;
        }
        if let Some(host) = &search_host {
            habibi.set("search", create_search_api(&lua, host.clone())?)?;
        }
        if manifest.capabilities.tools {
            let tools = lua.create_table()?;
            let tool_registry = registered_tools.clone();
            let tool_namespace = manifest.id.clone();
            tools.set(
                "register",
                lua.create_function(move |lua, (definition, handler): (LuaValue, Function)| {
                    let definition: ToolDefinition = lua.from_value(definition)?;
                    if !definition.name.starts_with(&format!("{tool_namespace}.")) {
                        return Err(mlua::Error::external(
                            "tool name must use the extension namespace",
                        ));
                    }
                    tool_registry
                        .lock()
                        .map_err(|_| mlua::Error::external("tool registry lock poisoned"))?
                        .push(RegisteredTool {
                            definition,
                            handler: lua.create_registry_value(handler)?,
                        });
                    Ok(())
                })?,
            )?;
            habibi.set("tools", tools)?;
        }

        if manifest.capabilities.web {
            let web = lua.create_table()?;
            let routes = registered_routes.clone();
            web.set(
                "route",
                lua.create_function(
                    move |lua, (method, path, handler): (String, String, Function)| {
                        if !path.starts_with('/') || path.contains("..") {
                            return Err(mlua::Error::external(
                                "extension route must be an absolute namespaced path",
                            ));
                        }
                        routes
                            .lock()
                            .map_err(|_| mlua::Error::external("route registry lock poisoned"))?
                            .push(RegisteredRoute {
                                method: method.to_uppercase(),
                                path,
                                handler: lua.create_registry_value(handler)?,
                            });
                        Ok(())
                    },
                )?,
            )?;
            habibi.set("web", web)?;
        }

        if manifest.capabilities.context {
            let context = lua.create_table()?;
            let hook_registry = context_hooks.clone();
            context.set(
                "register",
                lua.create_function(move |lua, (name, handler): (String, Function)| {
                    validate_hook_name(&name).map_err(mlua::Error::external)?;
                    hook_registry
                        .lock()
                        .map_err(|_| mlua::Error::external("context hook registry lock poisoned"))?
                        .push(RegisteredHook {
                            name,
                            handler: lua.create_registry_value(handler)?,
                        });
                    Ok(())
                })?,
            )?;
            habibi.set("context", context)?;
        }
        lua.globals().set("habibi", habibi)?;

        let entrypoint = directory.join("extension.lua");
        let source = fs::read_to_string(&entrypoint)
            .with_context(|| format!("failed to read {}", entrypoint.display()))?;
        lua.load(&source)
            .set_name(entrypoint.to_string_lossy())
            .exec()
            .with_context(|| format!("failed to initialize extension '{}'", manifest.id))?;

        let routes: Vec<RegisteredRoute> = registered_routes
            .lock()
            .map_err(|_| anyhow::anyhow!("extension route registry lock poisoned"))?
            .drain(..)
            .collect();
        let tools: Vec<RegisteredTool> = registered_tools
            .lock()
            .map_err(|_| anyhow::anyhow!("extension tool registry lock poisoned"))?
            .drain(..)
            .collect();
        let mut tool_names = HashSet::new();
        for tool in &tools {
            if !tool_names.insert(tool.definition.name.clone()) {
                bail!(
                    "extension '{}' registers duplicate tool '{}'",
                    manifest.id,
                    tool.definition.name
                );
            }
        }
        let mut route_names = HashSet::new();
        for route in &routes {
            if !route_names.insert((route.method.clone(), route.path.clone())) {
                bail!(
                    "extension '{}' registers duplicate route {} {}",
                    manifest.id,
                    route.method,
                    route.path
                );
            }
        }
        let mut context_hooks: Vec<RegisteredHook> = context_hooks
            .lock()
            .map_err(|_| anyhow::anyhow!("context hook registry lock poisoned"))?
            .drain(..)
            .collect();
        validate_unique_hooks(&manifest.id, "context", &context_hooks)?;
        context_hooks.sort_by(|left, right| left.name.cmp(&right.name));
        let static_files = load_static_files(directory, &manifest)?;
        let generation = extension_generation(&manifest_source, &source, &static_files);
        let execution_snapshot = snapshot_extension(directory)?;
        Ok(Self {
            manifest,
            generation,
            execution_snapshot,
            static_files,
            store,
            enabled: AtomicBool::new(enabled),
            state: Mutex::new(LuaState {
                lua,
                instruction_budget,
                routes,
                tools,
                context_hooks,
                filesystem_host,
                #[cfg(target_os = "linux")]
                process_host,
                studio_host,
                search_host,
            }),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn set_enabled(&self, enabled: bool) -> Result<()> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .set_extension_enabled(&self.manifest.id, enabled)?;
        self.enabled.store(enabled, Ordering::Release);
        Ok(())
    }

    pub fn handle_route(
        &self,
        method: &str,
        path: &str,
        mut request: RequestData,
    ) -> Result<Option<RouteOutcome>> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension '{}' lock poisoned", self.manifest.id))?;
        let Some((route, params)) = state.routes.iter().find_map(|route| {
            (route.method == method)
                .then(|| match_route(&route.path, path))
                .flatten()
                .map(|params| (route, params))
        }) else {
            return Ok(None);
        };
        request.path_params = params;
        state.instruction_budget.store(100, Ordering::Relaxed);
        let handler: Function = state.lua.registry_value(&route.handler)?;
        let request = state.lua.to_value(&request)?;
        let result: LuaValue = handler.call(request)?;
        Ok(Some(state.lua.from_value(result)?))
    }

    pub fn run_context_hooks(&self, trigger: &Event) -> Result<Vec<ContextHookExecution>> {
        if !self.is_enabled() {
            return Ok(Vec::new());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension '{}' lock poisoned", self.manifest.id))?;
        let mut executions = Vec::new();
        for hook in &state.context_hooks {
            state.instruction_budget.store(100, Ordering::Relaxed);
            let started = Instant::now();
            let attempted: Result<ContextContribution> = (|| {
                let handler: Function = state.lua.registry_value(&hook.handler)?;
                let argument = state.lua.to_value(trigger)?;
                let result: LuaValue = handler.call(argument).with_context(|| {
                    format!(
                        "extension '{}' context hook '{}' failed",
                        self.manifest.id, hook.name
                    )
                })?;
                Ok(state.lua.from_value(result)?)
            })();
            let (contribution, error) = match attempted {
                Ok(contribution) => (Some(contribution), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            };
            executions.push(ContextHookExecution {
                extension_id: self.manifest.id.clone(),
                hook: hook.name.clone(),
                duration_ms: started.elapsed().as_millis(),
                contribution,
                error,
            });
        }
        Ok(executions)
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        if !self.is_enabled() {
            return Vec::new();
        }
        self.state
            .lock()
            .map(|state| {
                state
                    .tools
                    .iter()
                    .map(|tool| tool.definition.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn execute_tool(&self, call: &ToolCall, context: &ToolContext) -> Result<ToolExecution> {
        if !self.is_enabled() {
            bail!("extension '{}' is disabled", self.manifest.id);
        }
        let isolated = Self::load(self.execution_snapshot.path(), self.store.clone())?;
        anyhow::ensure!(
            isolated.generation == self.generation,
            "extension '{}' changed after its tool catalog was pinned",
            self.manifest.id
        );
        isolated.execute_tool_in_state(call, context)
    }

    fn execute_tool_in_state(
        &self,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<ToolExecution> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension '{}' lock poisoned", self.manifest.id))?;
        let tool = state
            .tools
            .iter()
            .find(|tool| tool.definition.name == call.name)
            .with_context(|| {
                format!(
                    "extension '{}' does not own tool '{}'",
                    self.manifest.id, call.name
                )
            })?;
        state.instruction_budget.store(100, Ordering::Relaxed);
        if let Some(host) = &state.filesystem_host {
            host.clear_effects()?;
        }
        let _search_action = state
            .search_host
            .as_ref()
            .map(SearchHost::begin_action)
            .transpose()?;
        let _studio_action = state
            .studio_host
            .as_ref()
            .map(StudioHost::begin_action)
            .transpose()?;
        #[cfg(target_os = "linux")]
        let _process_action = state
            .process_host
            .as_ref()
            .map(ProcessHost::begin_action)
            .transpose()?;
        let attempted: Result<ToolExecution> = (|| {
            let handler: Function = state.lua.registry_value(&tool.handler)?;
            let arguments = state.lua.to_value(&call.arguments)?;
            let context = state.lua.to_value(context)?;
            let result: LuaValue = handler.call((arguments, context))?;
            Ok(state.lua.from_value(result)?)
        })();
        let mut host_events = state
            .filesystem_host
            .as_ref()
            .map(|host| host.take_effects())
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|effect| HostEffect {
                source: "host:filesystem",
                event: EventDraft {
                    event_type: effect.event_type,
                    payload: effect.payload,
                    idempotency_key: None,
                },
            })
            .collect::<Vec<_>>();
        #[cfg(target_os = "linux")]
        host_events.extend(
            state
                .process_host
                .as_ref()
                .map(|host| host.take_effects())
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .map(|effect| HostEffect {
                    source: "host:process",
                    event: EventDraft {
                        event_type: effect.event_type,
                        payload: effect.payload,
                        idempotency_key: None,
                    },
                }),
        );
        match attempted {
            Ok(mut execution) => {
                execution.host_events = host_events;
                Ok(execution)
            }
            Err(error) if host_events.is_empty() => Err(error),
            Err(error) => Ok(ToolExecution {
                result: Value::Null,
                events: Vec::new(),
                host_events,
                failure: Some(error.to_string()),
            }),
        }
    }

    pub fn static_file(&self, path: &str) -> Result<Option<(Vec<u8>, String)>> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let relative = if path == "/" || path.is_empty() {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if relative.split('/').any(|part| part == "..") {
            return Ok(None);
        }
        Ok(self.static_files.get(relative).cloned())
    }
}

pub struct ExtensionManager {
    directory: PathBuf,
    store: SharedEventStore,
    event_embeddings: SharedEventEmbeddingIndex,
    extensions: RwLock<HashMap<String, Arc<LoadedExtension>>>,
}

impl ExtensionManager {
    pub fn load(directory: &Path, store: SharedEventStore) -> Result<Self> {
        let mut extensions = HashMap::new();
        let event_embeddings = Arc::new(RwLock::new(None));
        if !directory.exists() {
            fs::create_dir_all(directory)?;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'))
                || !path.is_dir()
                || !path.join("extension.toml").exists()
            {
                continue;
            }
            let extension = Arc::new(LoadedExtension::load_with_embeddings(
                &path,
                store.clone(),
                event_embeddings.clone(),
            )?);
            if extensions
                .insert(extension.manifest.id.clone(), extension)
                .is_some()
            {
                bail!("duplicate extension id");
            }
        }
        Ok(Self {
            directory: directory.to_owned(),
            store,
            event_embeddings,
            extensions: RwLock::new(extensions),
        })
    }

    pub fn set_event_embedding_index(&self, index: Arc<EventEmbeddingIndex>) -> Result<()> {
        *self
            .event_embeddings
            .write()
            .map_err(|_| anyhow::anyhow!("event embedding index lock poisoned"))? = Some(index);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<Arc<LoadedExtension>> {
        self.extensions.read().ok()?.get(id).cloned()
    }

    pub fn reload(&self, id: &str) -> Result<Arc<LoadedExtension>> {
        let candidate = Arc::new(LoadedExtension::load_with_embeddings(
            &self.directory.join(id),
            self.store.clone(),
            self.event_embeddings.clone(),
        )?);
        if candidate.manifest.id != id {
            bail!("installed extension manifest id does not match directory '{id}'");
        }
        self.validate_reload_tool_names(id, &candidate)?;
        self.extensions
            .write()
            .map_err(|_| anyhow::anyhow!("extension registry lock poisoned"))?
            .insert(id.to_owned(), candidate.clone());
        Ok(candidate)
    }

    fn validate_reload_tool_names(
        &self,
        replaced_id: &str,
        candidate: &Arc<LoadedExtension>,
    ) -> Result<()> {
        let mut names = HashSet::new();
        let mut provider_names = HashSet::new();
        let mut extensions = self.snapshot();
        extensions.retain(|extension| extension.manifest.id != replaced_id);
        extensions.push(candidate.clone());
        for definition in extensions
            .iter()
            .flat_map(|extension| extension.tool_definitions())
        {
            if !names.insert(definition.name.clone()) {
                bail!("duplicate tool name '{}'", definition.name);
            }
            let provider_name = provider_tool_name(&definition.name);
            if !provider_names.insert(provider_name.clone()) {
                bail!("tool names collide after provider normalization: '{provider_name}'");
            }
        }
        Ok(())
    }

    fn snapshot(&self) -> Vec<Arc<LoadedExtension>> {
        let mut snapshot = self
            .extensions
            .read()
            .map(|extensions| extensions.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        snapshot.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        snapshot
    }

    pub fn run_context_hooks(&self, trigger: &Event) -> Result<Vec<ContextHookExecution>> {
        self.snapshot()
            .iter()
            .map(|extension| extension.run_context_hooks(trigger))
            .collect::<Result<Vec<_>>>()
            .map(|executions| executions.into_iter().flatten().collect())
    }

    pub fn tool_catalog_entries(&self) -> Vec<(ToolDefinition, Arc<LoadedExtension>)> {
        self.snapshot()
            .iter()
            .flat_map(|extension| {
                extension
                    .tool_definitions()
                    .into_iter()
                    .map(|definition| (definition, extension.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let Some(extension) = self.get(id) else {
            return Ok(false);
        };
        extension.set_enabled(enabled)?;
        Ok(true)
    }

    pub fn summaries(&self) -> Vec<ExtensionSummary> {
        let mut summaries = self
            .snapshot()
            .iter()
            .map(|extension| {
                let (route_count, tool_count, context_hook_count) = extension
                    .state
                    .lock()
                    .map(|state| {
                        (
                            state.routes.len(),
                            state.tools.len(),
                            state.context_hooks.len(),
                        )
                    })
                    .unwrap_or_default();
                let mut provides = Vec::new();
                if extension
                    .manifest
                    .web
                    .as_ref()
                    .and_then(|web| web.static_dir.as_ref())
                    .is_some()
                {
                    provides.push("Web interface".to_owned());
                }
                if route_count > 0 {
                    provides.push(format!("{route_count} HTTP API routes"));
                }
                if tool_count > 0 {
                    provides.push(format!("{tool_count} model tools"));
                }
                if extension.manifest.capabilities.kv {
                    provides.push("Namespaced KV storage".to_owned());
                }
                if extension.manifest.capabilities.events {
                    provides.push("Event history access".to_owned());
                }
                if extension.manifest.capabilities.filesystem {
                    provides.push("Granted filesystem access".to_owned());
                }
                if extension.manifest.capabilities.process {
                    provides.push("Sandboxed process execution".to_owned());
                }
                if extension.manifest.capabilities.studio {
                    provides.push("Scoped extension draft authoring".to_owned());
                }
                if extension.manifest.capabilities.search {
                    provides.push("External web search".to_owned());
                }
                if context_hook_count > 0 {
                    provides.push(format!("{context_hook_count} context hooks"));
                }
                let installation = ExtensionInstaller::new(self.directory.clone())
                    .metadata(&extension.manifest.id)
                    .ok();
                let filesystem_roots = extension
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| {
                        store
                            .extension_filesystem_roots(&extension.manifest.id)
                            .ok()
                    })
                    .unwrap_or_default();
                let process_executables = extension
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| {
                        store
                            .extension_process_executables(&extension.manifest.id)
                            .ok()
                    })
                    .unwrap_or_default();
                ExtensionSummary {
                    id: extension.manifest.id.clone(),
                    name: extension.manifest.name.clone(),
                    version: extension.manifest.version.clone(),
                    description: extension.manifest.description.clone(),
                    enabled: extension.is_enabled(),
                    capabilities: extension.manifest.capabilities.clone(),
                    provides,
                    installation,
                    filesystem_roots,
                    process_executables,
                    main_page: extension
                        .manifest
                        .web
                        .as_ref()
                        .and_then(|web| web.static_dir.as_ref())
                        .map(|_| format!("/extensions/{}/", extension.manifest.id)),
                }
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| left.id.cmp(&right.id));
        summaries
    }
}

#[derive(Debug, Serialize)]
pub struct ExtensionSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub capabilities: ExtensionCapabilities,
    pub provides: Vec<String>,
    pub installation: Option<InstallMetadata>,
    pub filesystem_roots: Vec<String>,
    pub process_executables: Vec<crate::store::ProcessExecutableGrant>,
    pub main_page: Option<String>,
}

fn create_search_api(lua: &Lua, host: SearchHost) -> mlua::Result<mlua::Table> {
    let api = lua.create_table()?;
    let configured = host.configured();
    api.set(
        "configured",
        lua.create_function(move |_, ()| Ok(configured))?,
    )?;
    api.set(
        "search",
        lua.create_function(move |lua, request: LuaValue| {
            let request: WebSearchRequest = lua.from_value(request)?;
            lua.to_value(&host.search(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    Ok(api)
}

fn create_studio_api(lua: &Lua, host: StudioHost) -> mlua::Result<mlua::Table> {
    let api = lua.create_table()?;
    let list_host = host.clone();
    api.set(
        "list",
        lua.create_function(move |lua, ()| {
            lua.to_value(&list_host.list_drafts().map_err(mlua::Error::external)?)
        })?,
    )?;
    let create_host = host.clone();
    api.set(
        "create",
        lua.create_function(move |lua, request: LuaValue| {
            let request: CreateDraftRequest = lua.from_value(request)?;
            lua.to_value(
                &create_host
                    .create_draft(request)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let files_host = host.clone();
    api.set(
        "list_files",
        lua.create_function(move |lua, draft_id: String| {
            lua.to_value(
                &files_host
                    .list_files(&draft_id)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let read_host = host.clone();
    api.set(
        "read",
        lua.create_function(move |lua, request: LuaValue| {
            let request: DraftFileRequest = lua.from_value(request)?;
            lua.to_value(
                &read_host
                    .read_file(request)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let write_host = host.clone();
    api.set(
        "write",
        lua.create_function(move |lua, request: LuaValue| {
            let request: WriteDraftFileRequest = lua.from_value(request)?;
            lua.to_value(
                &write_host
                    .write_file(request)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let directory_host = host.clone();
    api.set(
        "mkdir",
        lua.create_function(move |lua, request: LuaValue| {
            let request: CreateDraftDirectoryRequest = lua.from_value(request)?;
            directory_host
                .create_directory(request)
                .map_err(mlua::Error::external)
        })?,
    )?;
    api.set(
        "validate",
        lua.create_function(move |lua, draft_id: String| {
            lua.to_value(&host.validate(&draft_id).map_err(mlua::Error::external)?)
        })?,
    )?;
    Ok(api)
}

#[cfg(target_os = "linux")]
fn create_process_api(lua: &Lua, host: ProcessHost) -> mlua::Result<mlua::Table> {
    let api = lua.create_table()?;
    api.set(
        "run",
        lua.create_function(move |lua, request: LuaValue| {
            let request: ProcessRequest = lua.from_value(request)?;
            lua.to_value(&host.run(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    Ok(api)
}

fn create_files_api(lua: &Lua, host: FilesystemHost) -> mlua::Result<mlua::Table> {
    let files = lua.create_table()?;

    let operation = host.clone();
    files.set(
        "list",
        lua.create_function(move |lua, request: LuaValue| {
            let request: PathRequest = lua.from_value(request)?;
            lua.to_value(&operation.list(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "read",
        lua.create_function(move |lua, request: LuaValue| {
            let request: PathRequest = lua.from_value(request)?;
            lua.to_value(&operation.read(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "write",
        lua.create_function(move |lua, request: LuaValue| {
            let request: WriteRequest = lua.from_value(request)?;
            lua.to_value(&operation.write(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "patch",
        lua.create_function(move |lua, request: LuaValue| {
            let request: PatchRequest = lua.from_value(request)?;
            lua.to_value(&operation.patch(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "mkdir",
        lua.create_function(move |lua, request: LuaValue| {
            let request: PathRequest = lua.from_value(request)?;
            lua.to_value(
                &operation
                    .create_directory(request)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "move",
        lua.create_function(move |lua, request: LuaValue| {
            let request: MoveRequest = lua.from_value(request)?;
            lua.to_value(
                &operation
                    .move_path(request)
                    .map_err(mlua::Error::external)?,
            )
        })?,
    )?;
    let operation = host.clone();
    files.set(
        "delete",
        lua.create_function(move |lua, request: LuaValue| {
            let request: PathRequest = lua.from_value(request)?;
            lua.to_value(&operation.delete(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    files.set(
        "search",
        lua.create_function(move |lua, request: LuaValue| {
            let request: SearchRequest = lua.from_value(request)?;
            lua.to_value(&host.search(request).map_err(mlua::Error::external)?)
        })?,
    )?;
    Ok(files)
}

fn create_kv_api(
    lua: &Lua,
    extension_id: &str,
    store: SharedEventStore,
) -> mlua::Result<mlua::Table> {
    let kv = lua.create_table()?;
    let id = extension_id.to_owned();
    let get_store = store.clone();
    kv.set(
        "get",
        lua.create_function(move |lua, key: String| {
            let value = get_store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .kv_get(&id, &key)
                .map_err(mlua::Error::external)?;
            lua.to_value(&value)
        })?,
    )?;

    let id = extension_id.to_owned();
    let set_store = store.clone();
    kv.set(
        "set",
        lua.create_function(move |lua, (key, value): (String, LuaValue)| {
            let value: Value = lua.from_value(value)?;
            set_store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .kv_set(&id, &key, &value)
                .map_err(mlua::Error::external)
        })?,
    )?;

    let id = extension_id.to_owned();
    let delete_store = store.clone();
    kv.set(
        "delete",
        lua.create_function(move |_, key: String| {
            delete_store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .kv_delete(&id, &key)
                .map_err(mlua::Error::external)
        })?,
    )?;

    let id = extension_id.to_owned();
    kv.set(
        "list",
        lua.create_function(move |lua, prefix: Option<String>| {
            let values = store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .kv_list(&id, prefix.as_deref().unwrap_or(""))
                .map_err(mlua::Error::external)?;
            let values = values
                .into_iter()
                .map(|(key, value)| serde_json::json!({ "key": key, "value": value }))
                .collect::<Vec<_>>();
            lua.to_value(&values)
        })?,
    )?;
    Ok(kv)
}

fn create_events_api(
    lua: &Lua,
    store: SharedEventStore,
    event_embeddings: SharedEventEmbeddingIndex,
) -> mlua::Result<mlua::Table> {
    let events = lua.create_table()?;
    let get_store = store.clone();
    events.set(
        "get",
        lua.create_function(move |lua, event_id: String| {
            let value = get_store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .get_event(Some(&event_id), None)
                .map_err(mlua::Error::external)?;
            lua.to_value(&value)
        })?,
    )?;
    events.set(
        "semantic",
        lua.create_function(move |lua, query: LuaValue| {
            let query: SemanticEventQuery = lua.from_value(query)?;
            let index = event_embeddings
                .read()
                .map_err(|_| mlua::Error::external("event embedding index lock poisoned"))?
                .clone()
                .ok_or_else(|| mlua::Error::external("semantic event index is unavailable"))?;
            let matches = index
                .search(
                    &query.text,
                    query.before_sequence,
                    query.limit,
                    query.minimum_similarity,
                )
                .map_err(mlua::Error::external)?;
            lua.to_value(&matches)
        })?,
    )?;
    events.set(
        "query",
        lua.create_function(move |lua, query: LuaValue| {
            let query: EventQuery = lua.from_value(query)?;
            let values = store
                .lock()
                .map_err(|_| mlua::Error::external("event store lock poisoned"))?
                .query_events(&StoreEventQuery {
                    event_type: query.event_type,
                    event_type_prefix: query.prefix,
                    before_sequence: query.before_sequence,
                    after_sequence: query.after_sequence,
                    limit: query.limit.clamp(1, 1_000),
                    ..StoreEventQuery::default()
                })
                .map_err(mlua::Error::external)?;
            lua.to_value(&values)
        })?,
    )?;
    Ok(events)
}

fn extension_generation(
    manifest: &str,
    source: &str,
    static_files: &HashMap<String, (Vec<u8>, String)>,
) -> String {
    let mut hasher = Sha256::new();
    for bytes in [manifest.as_bytes(), source.as_bytes()] {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    let mut paths = static_files.keys().collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let (contents, content_type) = &static_files[path];
        for bytes in [
            path.as_bytes(),
            content_type.as_bytes(),
            contents.as_slice(),
        ] {
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
    }
    format!("{:x}", hasher.finalize())
}

fn snapshot_extension(directory: &Path) -> Result<tempfile::TempDir> {
    fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let target = destination.join(entry.file_name());
            if file_type.is_dir() {
                copy_tree(&entry.path(), &target)?;
            } else if file_type.is_file() {
                fs::copy(entry.path(), target)?;
            } else {
                bail!("extension snapshot cannot contain symlinks or special files");
            }
        }
        Ok(())
    }
    let snapshot = tempfile::tempdir()?;
    copy_tree(directory, snapshot.path())?;
    Ok(snapshot)
}

fn load_static_files(
    directory: &Path,
    manifest: &ExtensionManifest,
) -> Result<HashMap<String, (Vec<u8>, String)>> {
    let Some(static_dir) = manifest
        .web
        .as_ref()
        .and_then(|web| web.static_dir.as_deref())
    else {
        return Ok(HashMap::new());
    };
    let relative = Path::new(static_dir);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("extension '{}' has an unsafe static_dir", manifest.id);
    }
    let base = directory.canonicalize()?;
    let root = directory.join(relative).canonicalize()?;
    if !root.starts_with(&base) || !root.is_dir() {
        bail!("extension '{}' static_dir is invalid", manifest.id);
    }
    let mut files = HashMap::new();
    collect_static_files(&root, &root, &mut files, 0)?;
    Ok(files)
}

fn collect_static_files(
    root: &Path,
    directory: &Path,
    files: &mut HashMap<String, (Vec<u8>, String)>,
    total_size: usize,
) -> Result<usize> {
    let mut size = total_size;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("extension static directories cannot contain symbolic links");
        }
        if file_type.is_dir() {
            size = collect_static_files(root, &entry.path(), files, size)?;
        } else if file_type.is_file() {
            let contents = fs::read(entry.path())?;
            size = size.saturating_add(contents.len());
            if size > 32 * 1024 * 1024 {
                bail!("extension static files exceed the 32 MiB limit");
            }
            let relative = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let content_type = mime_guess::from_path(entry.path())
                .first_or_octet_stream()
                .essence_str()
                .to_owned();
            files.insert(relative, (contents, content_type));
        }
    }
    Ok(size)
}

fn validate_hook_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_' | '.')
        })
    {
        bail!("hook name must contain only lowercase letters, digits, '.', '-' and '_'");
    }
    Ok(())
}

fn validate_unique_hooks(extension_id: &str, kind: &str, hooks: &[RegisteredHook]) -> Result<()> {
    let mut names = HashSet::new();
    for hook in hooks {
        if !names.insert(&hook.name) {
            bail!(
                "extension '{extension_id}' registers duplicate {kind} hook '{}'",
                hook.name
            );
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &ExtensionManifest) -> Result<()> {
    if !matches!(manifest.api_version, 2 | 3) {
        bail!(
            "extension '{}' uses unsupported API version {}",
            manifest.id,
            manifest.api_version
        );
    }
    if manifest.capabilities.context && manifest.api_version != 3 {
        bail!(
            "extension '{}' must use API version 3 for formatted context hooks",
            manifest.id
        );
    }
    if manifest.id == "habibi" {
        bail!("extension id 'habibi' is reserved by the runtime");
    }
    if manifest.id.is_empty()
        || !manifest.id.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        bail!("extension id must contain only lowercase letters, digits, and hyphens");
    }
    if manifest.web.is_some() && !manifest.capabilities.web {
        bail!(
            "extension '{}' configures web without the web capability",
            manifest.id
        );
    }
    Ok(())
}

fn match_route(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_parts = pattern.trim_matches('/').split('/').collect::<Vec<_>>();
    let path_parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    if pattern_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();
    for (pattern, value) in pattern_parts.into_iter().zip(path_parts) {
        if let Some(name) = pattern.strip_prefix(':') {
            params.insert(name.to_owned(), value.to_owned());
        } else if pattern != value {
            return None;
        }
    }
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EventStore;

    #[test]
    fn matches_route_parameters() {
        let params =
            match_route("/api/sessions/:id/messages", "/api/sessions/abc/messages").unwrap();
        assert_eq!(params.get("id").map(String::as_str), Some("abc"));
        assert!(match_route("/one", "/two").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_api_runs_only_through_a_tool_and_records_a_host_effect() {
        if !crate::process::process_backend_available() {
            return;
        }
        let extension_directory = tempfile::tempdir().unwrap();
        fs::write(
            extension_directory.path().join("extension.toml"),
            "id = \"process\"\nname = \"Process\"\nversion = \"1.0.0\"\napi_version = 2\n[capabilities]\ntools = true\nfilesystem = true\nprocess = true\n",
        )
        .unwrap();
        fs::write(
            extension_directory.path().join("extension.lua"),
            concat!(
                "habibi.tools.register({ name = \"process.run\", description = \"Run\", input_schema = { type = \"object\" } }, function(arguments)\n",
                "  return { result = habibi.process.run(arguments) }\n",
                "end)\n",
            ),
        )
        .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let grants =
            crate::process::normalize_executable_grants(&std::collections::BTreeMap::from([(
                "printf".to_owned(),
                "/usr/bin/printf".to_owned(),
            )]))
            .unwrap();
        {
            let mut store = store.lock().unwrap();
            store
                .set_extension_filesystem_roots(
                    "process",
                    &[workspace.path().to_str().unwrap().to_owned()],
                )
                .unwrap();
            store
                .set_extension_process_executables("process", &grants)
                .unwrap();
        }
        let extension = LoadedExtension::load(extension_directory.path(), store).unwrap();
        let trigger = Event::new(
            "test.trigger",
            "test",
            uuid::Uuid::now_v7(),
            None,
            serde_json::json!({}),
        );
        let execution = extension
            .execute_tool(
                &ToolCall {
                    call_id: "call-1".into(),
                    name: "process.run".into(),
                    arguments: serde_json::json!({
                        "executable": "printf",
                        "args": ["%s", "literal;$(no-shell)"],
                        "cwd": workspace.path()
                    }),
                    argument_error: None,
                },
                &ToolContext {
                    current_event: trigger.clone(),
                    correlation_id: trigger.correlation_id,
                },
            )
            .unwrap();
        assert_eq!(execution.result["stdout"], "literal;$(no-shell)");
        assert_eq!(execution.host_events.len(), 1);
        assert_eq!(execution.host_events[0].source, "host:process");
        assert_eq!(
            execution.host_events[0].event.event_type,
            "process.execution.completed"
        );
    }

    #[test]
    fn tool_calls_use_isolated_lua_states() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 2\n[capabilities]\ntools = true\n").unwrap();
        fs::write(directory.path().join("extension.lua"),
            "counter = 0\nhabibi.tools.register({ name = 'example.count', description = 'Count', input_schema = { type = 'object' } }, function() counter = counter + 1 return { result = { counter = counter } } end)\n").unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extension = LoadedExtension::load(directory.path(), store).unwrap();
        let event = Event::new(
            "test.event",
            "test",
            uuid::Uuid::now_v7(),
            None,
            serde_json::json!({}),
        );
        let call = ToolCall {
            call_id: "call".into(),
            name: "example.count".into(),
            arguments: serde_json::json!({}),
            argument_error: None,
        };
        let context = ToolContext {
            current_event: event.clone(),
            correlation_id: event.correlation_id,
        };
        assert_eq!(
            extension.execute_tool(&call, &context).unwrap().result["counter"],
            1
        );
        assert_eq!(
            extension.execute_tool(&call, &context).unwrap().result["counter"],
            1
        );
    }

    #[test]
    fn filesystem_effect_survives_a_lua_failure() {
        let extension_directory = tempfile::tempdir().unwrap();
        fs::write(
            extension_directory.path().join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 2\n[capabilities]\ntools = true\nfilesystem = true\n",
        )
        .unwrap();
        fs::write(
            extension_directory.path().join("extension.lua"),
            concat!(
                "habibi.tools.register({ name = \"example.write\", description = \"Write\", input_schema = { type = \"object\" } }, function(arguments)\n",
                "  habibi.files.write(arguments)\n",
                "  return missing_function()\n",
                "end)\n",
            ),
        )
        .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        store
            .lock()
            .unwrap()
            .set_extension_filesystem_roots(
                "example",
                &[workspace.path().to_str().unwrap().to_owned()],
            )
            .unwrap();
        let extension = LoadedExtension::load(extension_directory.path(), store).unwrap();
        let trigger = Event::new(
            "test.trigger",
            "test",
            uuid::Uuid::now_v7(),
            None,
            serde_json::json!({}),
        );
        let output = workspace.path().join("created.txt");
        let execution = extension
            .execute_tool(
                &ToolCall {
                    call_id: "call-1".into(),
                    name: "example.write".into(),
                    arguments: serde_json::json!({
                        "path": output,
                        "content": "sentinel-content"
                    }),
                    argument_error: None,
                },
                &ToolContext {
                    current_event: trigger.clone(),
                    correlation_id: trigger.correlation_id,
                },
            )
            .unwrap();
        assert!(execution.failure.is_some());
        assert_eq!(execution.host_events.len(), 1);
        assert_eq!(
            execution.host_events[0].event.event_type,
            "workspace.file.created"
        );
        assert_eq!(fs::read_to_string(output).unwrap(), "sentinel-content");
    }

    #[test]
    fn tool_suggestion_api_is_not_available() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 2\n[capabilities]\ntools = true\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("extension.lua"),
            "habibi.tools.suggest('legacy', function() return {} end)",
        )
        .unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let error = LoadedExtension::load(directory.path(), store)
            .err()
            .unwrap();
        let error = format!("{error:#}");
        assert!(error.contains("suggest"), "{error}");
    }

    #[test]
    fn context_hooks_can_get_events_by_id() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 3\n[capabilities]\ncontext = true\nevents = true\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("extension.lua"),
            "habibi.context.register(\"retrieve\", function(trigger)\n  local event = habibi.events.get(trigger.causation_id)\n  return { content = habibi.json.encode(event) }\nend)\n",
        )
        .unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let parent = Event::new(
            "test.parent",
            "test",
            uuid::Uuid::now_v7(),
            None,
            serde_json::json!({}),
        );
        store.lock().unwrap().append(&parent).unwrap();
        let extension = LoadedExtension::load(directory.path(), store).unwrap();
        let trigger = Event::new(
            "test.trigger",
            "test",
            parent.correlation_id,
            Some(parent.id),
            serde_json::json!({}),
        );
        let contribution = extension.run_context_hooks(&trigger).unwrap()[0]
            .contribution
            .clone()
            .unwrap();
        assert!(contribution.content.contains(&parent.id.to_string()));
        assert!(contribution.content.contains("test.parent"));
    }

    #[test]
    fn context_hooks_are_ordered_and_fail_independently() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("extension.toml"),
            "id = \"example\"\nname = \"Example\"\nversion = \"1.0.0\"\napi_version = 3\n[capabilities]\ncontext = true\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("extension.lua"),
            concat!(
                "habibi.context.register(\"z-failing\", function() error(\"boom\") end)\n",
                "habibi.context.register(\"a-good\", function() return { content = \"\" } end)\n",
            ),
        )
        .unwrap();
        let store = EventStore::open(":memory:").unwrap().shared();
        let extension = LoadedExtension::load(directory.path(), store).unwrap();
        let trigger = Event::new(
            "test.trigger",
            "test",
            uuid::Uuid::now_v7(),
            None,
            serde_json::json!({}),
        );
        let executions = extension.run_context_hooks(&trigger).unwrap();
        assert_eq!(executions[0].hook, "a-good");
        assert!(executions[0].contribution.is_some());
        assert_eq!(executions[1].hook, "z-failing");
        assert!(executions[1].error.is_some());
    }
}
