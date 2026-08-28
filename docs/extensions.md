# Habibi extensions

Extensions are Lua programs loaded from subdirectories of `HABIBI_EXTENSIONS_DIR`. Each
extension has its own HTTP namespace and KV namespace. The first API version is deliberately
small and synchronous.

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
An extension with static web content receives an **Open** link to `/extensions/{id}/`.

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
  },
  continuation = "required"
}, function(arguments, context)
  return {
    result = { item = lookup(arguments.id) },
    events = {}
  }
end)
```

`continuation` is `required` when the model needs the result, or `terminal` for outward effects
such as sending a message. A handler may return namespaced effect events. Habibi records every
proposal, start, result, failure, and batch barrier while preserving the reaction correlation ID.
Calls emitted in one model turn form a batch; all results are gathered and returned in one
continuation in original call order.

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

Set `react = true` to invoke the model after the event is appended. The extension registers a
context mapper; user-visible effects are performed by model tools rather than an automatic
response mapper:

```lua
habibi.reactions.context(function(trigger)
  return habibi.array({
    { role = "user", content = trigger.payload.content }
  })
end)
```

All model invocations, actions, tool effects, and continuations share the trigger's correlation
ID and are connected through causation IDs and batch result references.

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
