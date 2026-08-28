# Habibi extensions

Extensions are versioned Lua packages installed beneath `HABIBI_EXTENSIONS_DIR`. Each extension
has its own HTTP namespace and KV namespace. The first API version is deliberately small and
synchronous.

Install from a local package or Git repository:

```sh
habibi install ./my-extension
habibi install https://github.com/HabibiAssistant/extensions.git --subdir chat
habibi install https://github.com/example/plugin.git --ref v1.2.0
habibi update chat
```

Habibi copies packages rather than executing them in place, rejects symbolic links and unsafe
subdirectories, validates them in an isolated Lua runtime, and records source/revision/version and
a content hash in `.habibi-install.json`. Content changes require a semantic version increase.

## Layout

```text
extensions/example/
  extension.toml
  extension.lua
  web/
    index.html
```

## Manifest

```toml
id = "example"
name = "Example"
version = "0.1.0"
description = "What this extension adds to Habibi."
api_version = 1

[capabilities]
web = true
kv = true
events = true
tools = true

[web]
static_dir = "web"
```

Capabilities are optional and default to false. An extension receives only the corresponding
host APIs. Lua runs without the `io`, `os`, `package`, or `debug` standard libraries.

Habibi also infers provided features from registered routes, static web content, and reaction
handlers. These details appear at `/extensions`, where extensions can be enabled or disabled.
An extension with static web content receives an **Open** link to its `/extensions/{id}/` page on
the isolated extension-web origin (`HABIBI_EXTENSION_BIND`, port 8788 by default). Core management
remains on port 8787 so extension JavaScript cannot directly invoke privileged update endpoints.

## Web routes

Every route is mounted below `/extensions/{extension-id}`:

```lua
habibi.web.route("POST", "/api/items/:item_id", function(request)
  return {
    status = 201,
    json = { id = request.path_params.item_id }
  }
end)
```

Request fields:

```text
method, path, path_params, query, headers, body, json
```

A response may contain `json`, or `body` with an optional `content_type`.

Static files configured by `web.static_dir` are served when no dynamic GET route matches.
Paths cannot escape the configured directory.

## Model tools and actions

Extensions register model-callable tools in their own namespace:

```lua
habibi.tools.register({
  name = "example.lookup",
  description = "Look up an example item.",
  input_schema = {
    type = "object",
    properties = { id = { type = "string" } },
    required = { "id" }
  }
}, function(arguments, context)
  return {
    result = { item = lookup(arguments.id) },
    events = {}
  }
end)
```

A handler may return namespaced effect events. Habibi records every action request and result as
an event, records execution diagnostics as logs, and gathers all calls from one processed event
into a batch. The resulting `action.batch.completed` event is queued with all results in original
call order and is processed as the next event. There is no internal turn limit: the reaction
settles when the event queue is empty.

## Emitting events and requesting reactions

A route can ask the host to append one event:

```lua
return {
  status = 201,
  emit = {
    type = "example.item.created",
    payload = { item_id = habibi.id() }
  }
}
```

Every emitted event enters the reactor after it is appended. The extension registers a context
mapper; user-visible effects are performed by model tools:

```lua
habibi.reactions.context(function(trigger)
  return habibi.array({
    { role = "user", content = trigger.payload.content }
  })
end)
```

All action requests, results, tool effects, batch barriers, and operational logs share the
trigger's correlation ID and are connected through causation IDs and result references.

## Event queries

```lua
local recent = habibi.events.query({
  prefix = "example.",
  limit = 100
})

local messages = habibi.events.query({
  type = "example.message.created",
  limit = 100
})
```

Results are chronological and include `sequence` plus the complete core event envelope.
Each query is capped at 1,000 events. Use `before_sequence` or `after_sequence` to paginate
without relying on timestamps.

## KV storage

```lua
habibi.kv.set("preferences", { theme = "dark" })
local preferences = habibi.kv.get("preferences")
local entries = habibi.kv.list("preference/")
habibi.kv.delete("preferences")
```

Values must be JSON-compatible. The host always supplies the extension namespace; extension
code cannot select another extension's namespace.

KV is intended for incidental mutable state such as preferences, drafts, and caches. Domain
history should generally remain event-sourced.

## JSON arrays

Empty Lua tables are ambiguous when converted to JSON. Use `habibi.array({})` when an empty
table must serialize as an array or deserialize into a Rust list.
