# Habibi extensions

Extensions are versioned Lua packages installed beneath `HABIBI_EXTENSIONS_DIR`. Each extension
has its own HTTP namespace and KV namespace. API version 2 provides typed context hooks and
separate tool-suggestion hooks while remaining deliberately small and synchronous.

Install from a local package or Git repository:

```sh
habibi install ./my-extension
habibi install https://github.com/HabibiAssistant/extensions.git --subdir chat
habibi install https://github.com/example/plugin.git --ref v1.2.0
habibi update chat
```

Habibi copies packages rather than executing them in place, rejects symbolic links and unsafe
subdirectories, runs an automatic security/privacy scan, validates them in an isolated Lua runtime,
and records source/revision/version, content hash, capabilities, and the scan report in
`.habibi-install.json`. Blocking scan findings abort the operation before the package can be enabled;
warnings are displayed for review. Content changes require a semantic version increase.

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
api_version = 2

[capabilities]
web = true
kv = true
events = true
tools = true
context = true
filesystem = true
process = true

[web]
static_dir = "web"
```

Capabilities are optional and default to false. An extension receives only the corresponding
host APIs. Lua runs without the `io`, `os`, `package`, or `debug` standard libraries. API version 2
replaces the former reaction-context callback; packages must use `habibi.context.register` and may
use `habibi.tools.suggest`. The obsolete callback is not retained.

Habibi also infers provided features from registered routes, static web content, context hooks,
and tool suggestion hooks. These details appear at `/extensions`, where extensions can be enabled
or disabled.
An extension with static web content receives an **Open** link to `/extensions/{id}/`. Installed
extensions are fully trusted local code; capabilities make behavior visible during review and
control which Lua host APIs are exposed, but they are not a hostile-code security boundary.

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

Only `habibi.tools.search`, event-relevant extension suggestions, tools discovered in the current
causal chain, and tools already used in that chain are advertised to a model invocation. Extensions
can suggest their own tools separately from context creation:

```lua
habibi.tools.suggest("example-created", function(trigger)
  if trigger.event_type ~= "example.item.created" then return habibi.array({}) end
  return habibi.array({{
    tool = "example.lookup",
    reason = "New example items commonly need enrichment."
  }})
end)
```

Suggestions are discovery hints, not authorization. Dangerous tools must enforce confirmation and
argument policy when executed. Tool definitions and handlers are pinned to one validated catalog
generation for the complete reaction chain. Tool advertisements, calls, outcomes, schema-token
estimates, and execution durations are recorded in logs and aggregated at `/stats`.

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

Every event emitted by an extension route enters the reactor after it is appended. Internal action
request, effect, and result events remain durable facts, but they are not each sent separately to
the model; their ordered `action.batch.completed` aggregate is the next model input. Core supplies a stable system prompt
and the current event. An extension with the `context` capability may contribute its own event
references or message projections. Hooks run deterministically by extension ID and hook name;
failed hooks are logged and skipped without stopping other extensions.

```lua
habibi.context.register("example-history", function(trigger)
  if not trigger.payload.content then return { items = habibi.array({}) } end
  return {
    items = habibi.array({{
      type = "message",
      role = "user",
      content = trigger.payload.content,
      source_event_id = trigger.id
    }})
  }
end)
```

An event contribution uses `{ type = "event", event_id = "..." }`. Message roles are `user` or
`assistant` and every message must reference an existing immutable source event. Extensions can
select and render only their own returned contributions; they cannot rewrite core input or another
extension's contribution. Exact duplicate projections from one hook are omitted, conflicting
projections are rejected, and each hook is bounded to 500 items and 2 MiB of rendered input.
User-visible effects are performed by model tools.

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

## Granted filesystem access

The `filesystem` capability exposes `habibi.files`, but grants no paths by itself. Users grant
existing absolute directories from the Extensions page. An extension cannot create or broaden its
own grants. Empty grants deny every operation.

```lua
local file = habibi.files.read({ path = "/home/user/project/README.md" })
local changed = habibi.files.patch({
  path = file.path,
  old_text = "old text",
  new_text = "new text",
  expected_sha256 = file.sha256
})
```

Available host operations are `list`, `read`, `search`, `write`, `patch`, `mkdir`, `move`, and
`delete`. Paths must be absolute, remain beneath a granted canonical root, and contain no `.` or
`..` components. Capability-based directory handles confine actual reads and mutations; symbolic
links and special files are not followed. Reads and writes are limited to 2 MiB. Search is bounded
by query length, depth, entries, files, bytes, matches, and output preview size.

Creating a file requires a missing destination. Replacing or patching an existing file requires the
exact SHA-256 returned by `read`; stale writes fail without changing the target. Writes use a synced
temporary file and atomic rename. Deletes are nonrecursive and cannot delete a granted root. Moves
cannot cross granted roots or overwrite intentionally existing destinations.

Filesystem mutations are serialized within one loaded extension generation. Core—not Lua—records
host-authored `workspace.*` mutation effects, including when Lua fails after the mutation. Effect
payloads contain paths, hashes, and sizes, never file contents or patch text. Action requests,
action results, and exact model logs still retain tool arguments/results under Habibi's existing
observability policy; filesystem grants are therefore a scope boundary, not a secret-content
redaction feature.

## Sandboxed process execution

The Linux-only `process` capability exposes `habibi.process.run` only while a registered tool handler
is executing. Initialization, routes, context hooks, and suggestion hooks cannot run processes. The
extension must have both an exact executable grant and a filesystem root containing the requested
working directory.

```lua
local outcome = habibi.process.run({
  executable = "git",
  args = habibi.array({ "status", "--porcelain=v1" }),
  cwd = "/home/user/project",
  timeout_ms = 30000
})
```

Users configure executable aliases on the Extensions page. Grants accept only canonical executable
native ELF files, store device/inode identity and SHA-256, and are verified on every invocation. The
verified bytes are copied into a sealed memory file before launch. There is no executable path
lookup, implicit shell evaluation, script/shebang support, caller environment, stdin, detached mode,
or network. Explicitly granting a native interpreter grants its normal argv authority.
Arguments are literal argv entries, limited to 128 entries and 64 KiB total.

Each run uses Bubblewrap namespaces and a delegated cgroup v2 leaf. The sandbox receives a minimal
runtime filesystem plus one selected filesystem grant mounted read-write. Stdout and stderr are
drained concurrently and capped at 1 MiB each. Timeout defaults to 30 seconds and is capped at 120
seconds. Completion, timeout, and output overflow kill the whole cgroup. Execution fails closed when
Bubblewrap or delegated cgroup v2 is unavailable.

Core emits a host-authored `process.execution.completed` effect with executable alias/hash, cwd,
outcome, duration, exit status, and byte counts—never argv or output. Tool arguments, returned output,
action results, and exact model logs remain durable. Do not use process tools for secrets.

## JSON arrays

Empty Lua tables are ambiguous when converted to JSON. Use `habibi.array({})` when an empty
table must serialize as an array or deserialize into a Rust list.
