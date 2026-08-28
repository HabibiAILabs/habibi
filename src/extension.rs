use std::{
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use mlua::{
    Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, RegistryKey, StdLib, Value as LuaValue,
    VmState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    event::{ConversationMessage, Event},
    store::{SharedEventStore, StoreEventQuery},
    tool::{ToolCall, ToolContext, ToolDefinition, ToolExecution},
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
    #[serde(default)]
    pub react: bool,
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

struct RegisteredTool {
    definition: ToolDefinition,
    handler: RegistryKey,
}

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
    context_handler: Option<RegistryKey>,
}

pub struct LoadedExtension {
    pub manifest: ExtensionManifest,
    base_dir: PathBuf,
    store: SharedEventStore,
    enabled: AtomicBool,
    state: Mutex<LuaState>,
}

impl LoadedExtension {
    fn load(directory: &Path, store: SharedEventStore) -> Result<Self> {
        let manifest_path = directory.join("extension.toml");
        let manifest: ExtensionManifest = toml::from_str(
            &fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?,
        )
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
        let context_handler = Arc::new(Mutex::new(None));
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

        if manifest.capabilities.kv {
            habibi.set("kv", create_kv_api(&lua, &manifest.id, store.clone())?)?;
        }
        if manifest.capabilities.events {
            habibi.set("events", create_events_api(&lua, store.clone())?)?;
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

        let reactions = lua.create_table()?;
        let context_slot = context_handler.clone();
        reactions.set(
            "context",
            lua.create_function(move |lua, handler: Function| {
                *context_slot
                    .lock()
                    .map_err(|_| mlua::Error::external("reaction registry lock poisoned"))? =
                    Some(lua.create_registry_value(handler)?);
                Ok(())
            })?,
        )?;
        habibi.set("reactions", reactions)?;
        lua.globals().set("habibi", habibi)?;

        let entrypoint = directory.join("extension.lua");
        let source = fs::read_to_string(&entrypoint)
            .with_context(|| format!("failed to read {}", entrypoint.display()))?;
        lua.load(&source)
            .set_name(entrypoint.to_string_lossy())
            .exec()
            .with_context(|| format!("failed to initialize extension '{}'", manifest.id))?;

        let routes = registered_routes
            .lock()
            .map_err(|_| anyhow::anyhow!("extension route registry lock poisoned"))?
            .drain(..)
            .collect();
        let tools = registered_tools
            .lock()
            .map_err(|_| anyhow::anyhow!("extension tool registry lock poisoned"))?
            .drain(..)
            .collect();
        let context_handler = context_handler
            .lock()
            .map_err(|_| anyhow::anyhow!("context registry lock poisoned"))?
            .take();
        Ok(Self {
            manifest,
            base_dir: directory.to_owned(),
            store,
            enabled: AtomicBool::new(enabled),
            state: Mutex::new(LuaState {
                lua,
                instruction_budget,
                routes,
                tools,
                context_handler,
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

    pub fn compile_context(&self, trigger: &Event) -> Result<Vec<ConversationMessage>> {
        if !self.is_enabled() {
            bail!("extension '{}' is disabled", self.manifest.id);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension '{}' lock poisoned", self.manifest.id))?;
        let key = state
            .context_handler
            .as_ref()
            .context("extension did not register a reaction context handler")?;
        state.instruction_budget.store(100, Ordering::Relaxed);
        let handler: Function = state.lua.registry_value(key)?;
        let trigger = state.lua.to_value(trigger)?;
        let result: LuaValue = handler.call(trigger)?;
        Ok(state.lua.from_value(result)?)
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

    pub fn execute_tool(
        &self,
        call: &ToolCall,
        context: &ToolContext,
    ) -> Result<Option<ToolExecution>> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("extension '{}' lock poisoned", self.manifest.id))?;
        let Some(tool) = state
            .tools
            .iter()
            .find(|tool| tool.definition.name == call.name)
        else {
            return Ok(None);
        };
        state.instruction_budget.store(100, Ordering::Relaxed);
        let handler: Function = state.lua.registry_value(&tool.handler)?;
        let arguments = state.lua.to_value(&call.arguments)?;
        let context = state.lua.to_value(context)?;
        let result: LuaValue = handler.call((arguments, context))?;
        Ok(Some(state.lua.from_value(result)?))
    }

    pub fn static_file(&self, path: &str) -> Result<Option<(Vec<u8>, String)>> {
        if !self.is_enabled() {
            return Ok(None);
        }
        let Some(web) = &self.manifest.web else {
            return Ok(None);
        };
        let Some(static_dir) = &web.static_dir else {
            return Ok(None);
        };
        let static_dir = Path::new(static_dir);
        if static_dir.is_absolute()
            || static_dir.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("extension '{}' has an unsafe static_dir", self.manifest.id);
        }
        let relative = if path == "/" || path.is_empty() {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if relative.split('/').any(|part| part == "..") {
            return Ok(None);
        }
        let base = self.base_dir.canonicalize()?;
        let root = self.base_dir.join(static_dir).canonicalize()?;
        if !root.starts_with(&base) {
            bail!(
                "extension '{}' static_dir escapes its directory",
                self.manifest.id
            );
        }
        let file = self.base_dir.join(static_dir).join(relative);
        let Ok(file) = file.canonicalize() else {
            return Ok(None);
        };
        if !file.starts_with(root) || !file.is_file() {
            return Ok(None);
        }
        let content_type = mime_guess::from_path(&file)
            .first_or_octet_stream()
            .essence_str()
            .to_owned();
        Ok(Some((fs::read(file)?, content_type)))
    }
}

pub struct ExtensionManager {
    extensions: HashMap<String, Arc<LoadedExtension>>,
}

impl ExtensionManager {
    pub fn load(directory: &Path, store: SharedEventStore) -> Result<Self> {
        let mut extensions = HashMap::new();
        if !directory.exists() {
            fs::create_dir_all(directory)?;
        }
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if !path.is_dir() || !path.join("extension.toml").exists() {
                continue;
            }
            let extension = Arc::new(LoadedExtension::load(&path, store.clone())?);
            if extensions
                .insert(extension.manifest.id.clone(), extension)
                .is_some()
            {
                bail!("duplicate extension id");
            }
        }
        Ok(Self { extensions })
    }

    pub fn get(&self, id: &str) -> Option<Arc<LoadedExtension>> {
        self.extensions.get(id).cloned()
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.extensions
            .values()
            .flat_map(|extension| extension.tool_definitions())
            .collect()
    }

    pub fn execute_tool(&self, call: &ToolCall, context: &ToolContext) -> Result<ToolExecution> {
        for extension in self.extensions.values() {
            if let Some(result) = extension.execute_tool(call, context)? {
                return Ok(result);
            }
        }
        bail!("unknown extension tool '{}'", call.name)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let Some(extension) = self.extensions.get(id) else {
            return Ok(false);
        };
        extension.set_enabled(enabled)?;
        Ok(true)
    }

    pub fn summaries(&self) -> Vec<ExtensionSummary> {
        let mut summaries = self
            .extensions
            .values()
            .map(|extension| {
                let (route_count, tool_count, reactions) = extension
                    .state
                    .lock()
                    .map(|state| {
                        (
                            state.routes.len(),
                            state.tools.len(),
                            state.context_handler.is_some(),
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
                if reactions {
                    provides.push("Model reactions".to_owned());
                }
                ExtensionSummary {
                    id: extension.manifest.id.clone(),
                    name: extension.manifest.name.clone(),
                    version: extension.manifest.version.clone(),
                    description: extension.manifest.description.clone(),
                    enabled: extension.is_enabled(),
                    capabilities: extension.manifest.capabilities.clone(),
                    provides,
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
    pub main_page: Option<String>,
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

fn create_events_api(lua: &Lua, store: SharedEventStore) -> mlua::Result<mlua::Table> {
    let events = lua.create_table()?;
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

fn validate_manifest(manifest: &ExtensionManifest) -> Result<()> {
    if manifest.api_version != 1 {
        bail!(
            "extension '{}' uses unsupported API version {}",
            manifest.id,
            manifest.api_version
        );
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

    #[test]
    fn matches_route_parameters() {
        let params =
            match_route("/api/sessions/:id/messages", "/api/sessions/abc/messages").unwrap();
        assert_eq!(params.get("id").map(String::as_str), Some("abc"));
        assert!(match_route("/one", "/two").is_none());
    }
}
