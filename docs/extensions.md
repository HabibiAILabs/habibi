# Habibi extensions

Extensions are versioned Lua packages installed beneath `HABIBI_EXTENSIONS_DIR`. Each extension
has its own HTTP namespace and KV namespace. API version 3 provides formatted context hooks;
API version 4 provides global-boundary filesystem and process APIs.

Install from a local package or Git repository:

```sh
habibi install ./my-extension
habibi install https://github.com/HabibiAILabs/extensions.git --subdir chat
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
api_version = 4

[capabilities]
web = true
kv = true
events = true
tools = true
context = true
filesystem = true
process = true
search = true

[web]
static_dir = "web"
```

Capabilities are optional and default to false. An extension receives only the corresponding
host APIs. Lua runs without the `io`, `os`, `package`, or `debug` standard libraries. API version 3
or newer uses `habibi.context.register` for per-event model context. Filesystem/process capabilities
require API version 4. Tool discovery is core-owned semantic retrieval over registered tool descriptions and input fields.

Habibi also infers provided features from registered routes, static web content, and context hooks. These details appear at `/extensions`, where extensions can be enabled
or disabled.
An extension receives an **Open** link only when it registers a home with `habibi.web.home`. Installed
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

An extension may designate one optional application home page and icon during initialization:

```lua
habibi.web.home({ path = "/", icon = "/icon.svg", title = "Example App" })
```

Both paths are extension-relative absolute paths. `title` optionally gives the app a display name distinct from the extension package name. A registered home appears as an application card
on Habibi's homepage and as an **Open** action on the Extensions page. Habibi opens it inside the
shared `/apps/{extension-id}` shell so global navigation remains visible. Extensions are trusted,
same-origin applications; the shell uses an iframe for layout isolation, not a security boundary.
Extensions without a registered icon receive a generated initials mark.

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

A handler may return namespaced effect events. Habibi records every action request, result, and
effect as an immutable event and execution diagnostics as logs. All calls from one model response
form one action group and execute concurrently. The reserved `_habibi_delivery` input property is
injected into advertised schemas and stripped before validation/execution, so handlers never receive
it. Values are `asap` and `batch`; omission defaults to ASAP for one call and batch for multiple calls.
Invalid values use that same deterministic default.

Core validates every proposed call against the pinned `input_schema` before invoking any handler.
Action groups are atomic at this boundary: if one call is invalid, none execute and no action events
are written. The model receives structured call index, tool, instance path, and validation message as
temporary correction input. It may retry the complete group up to three times. Validation attempts
and exhaustion are operational logs, not events. Malformed argument JSON and tool names outside the advertised surface follow the same correction path.

ASAP result events enter the durable model inbox individually as execution finishes. Batch results
do not enter individually; `actions.completed` exposes those results once, in original call order.
The completion event is always persisted and is enqueued only when batched results exist. Failed
results use the selected delivery mode. Tools cannot suppress or terminate result delivery.

For every claimed event, core semantically ranks registered tools from a bounded current-event projection. Extension-provided system context does not alter tool retrieval. Tools actually called earlier in the correlation have priority, and semantic matches fill a final maximum of 12 tools. `habibi.tools.search` is indexed like any other tool and uses the same semantic index if selected. Dangerous tools must enforce confirmation and argument policy when executed. Tool definitions and handlers are pinned to one validated catalog
generation for each action group. Tool advertisements, calls, outcomes, schema-token
estimates, and execution durations are recorded in logs and aggregated at `/stats`.

## Emitting events and dispatching model work

A route can ask the host to append one event:

```lua
return {
  status = 201,
  emit = {
    type = "example.item.created",
    idempotency_key = request.json.request_id,
    payload = { item_id = request.json.request_id }
  }
}
```

When `emit` is present, core validates the namespace, appends the event and durable inbox row in one
SQLite transaction, normalizes the response to `202 Accepted`, and adds `event_id`, `correlation_id`,
and `sequence`. If `idempotency_key` is present, the JSON response and acceptance metadata are stored
atomically; retries of the same extension/type/key return the original acceptance without a new event. Model processing continues independently of the request. Core supplies a stable system
prompt and the immutable claimed event. An extension with the `context` capability may contribute
formatted context text. Hooks run anew for every claimed event, deterministically by extension ID
and hook name; failed hooks are logged and skipped without stopping peers.

```lua
habibi.context.register("example-history", function(trigger)
  if not trigger.payload.content then return { content = "" } end
  return {
    content = "Relevant example data:\n" .. habibi.json.encode(trigger.payload)
  }
end)
```

A context hook returns one extension-formatted UTF-8 `content` string. Core does not assign message
roles, fetch source events, or reinterpret its contents. It places every non-empty contribution in
a labeled, delimited section of the invocation's system message. The one immutable current event is
the invocation's only user message. Each hook and the combined extension context are bounded to
2 MiB. User-visible effects are performed by model tools.

All action requests, results, tool effects, batch barriers, and operational logs share the
trigger's correlation ID and are connected through causation IDs and result references.

## Event access

```lua
local parent = habibi.events.get(trigger.causation_id)

local recent = habibi.events.query({
  prefix = "example.",
  limit = 100
})

local messages = habibi.events.query({
  type = "example.message.created",
  limit = 100
})

local related = habibi.events.semantic({
  text = "formatted retrieval query",
  before_sequence = trigger_sequence,
  limit = 20,
  minimum_similarity = 0.50
})
```

`get` returns the complete stored event or `nil` when the ID does not exist. Query results are
chronological and include `sequence` plus the complete core event envelope. Each query is capped
at 1,000 events. Use `before_sequence` or `after_sequence` to paginate without relying on timestamps.

`semantic` uses the explicitly installed pinned local embedding model, scans at most the newest
10,000 prior events, and returns model/revision metadata, candidate count, and at most 20
`{ event, score, rank }` entries in `matches`. It performs no download
or network request. Queries are bounded to 16 KiB. Extensions remain responsible for merging,
filtering, ordering, deduplicating, and formatting results.

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
history should generally remain event-sourced. The Extensions page links to a read-only core KV
Explorer for each KV-capable extension.

## Typed configuration

An extension may provide a JSON Schema from its package:

```toml
[config]
schema = "config.schema.json"
```

Habibi exposes a schema-validated editor at `/admin/extensions/{extension-id}/config` and stores the
complete JSON value under the extension namespace. Lua reads it with `habibi.config.get()`. Invalid
values are rejected atomically. Extensions may instead provide their own configuration application,
as Soul does; configuration controls do not appear inline on extension cards.

## Global-boundary filesystem access

The `filesystem` capability exposes `habibi.files`, but grants no paths by itself. Core Settings
hold global include and exclude absolute path patterns. Every filesystem-capable extension shares
this maximum boundary. The most specific match wins and includes win equal-specificity ties, allowing
specific includes to override `*` exclusions. Empty includes deny every operation.

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
`delete`. Paths must be absolute, remain beneath a globally included canonical root and outside exclusions, and contain no `.` or
`..` components. Capability-based directory handles confine actual reads and mutations; symbolic
links and special files are not followed. Reads and writes are limited to 2 MiB. Search is bounded
by query length, depth, entries, files, bytes, matches, and output preview size.

Creating a file requires a missing destination. Replacing or patching an existing file requires the
exact SHA-256 returned by `read`; stale writes fail without changing the target. Writes use a synced
temporary file and atomic rename. Deletes are nonrecursive and cannot delete an exact included root. Moves require both paths to be
allowed, cannot cross filesystems, and never overwrite an existing destination.

Filesystem mutations are serialized within one loaded extension generation. Core—not Lua—records
host-authored `workspace.*` mutation effects, including when Lua fails after the mutation. Effect
payloads contain paths, hashes, and sizes, never file contents or patch text. Action requests,
action results, and exact model logs still retain tool arguments/results under Habibi's existing
observability policy; filesystem boundaries are therefore a scope boundary, not a secret-content
redaction feature.

## Sandboxed process execution

The Linux-only API-version-4 `process` capability exposes `habibi.process.run` only while a registered tool handler
is executing. Initialization, routes, and context hooks cannot run processes. The
requested program and working directory must both pass the global core boundary policy.

```lua
local outcome = habibi.process.run({
  program = "git", -- or its approved absolute path
  args = habibi.array({ "status", "--porcelain=v1" }),
  cwd = "/home/user/project",
  timeout_ms = 30000,
  filesystem_access = "read_only" -- defaults to read_write
})
```

Users configure global program include/exclude patterns on the Settings page. Exact entries are
canonical native ELF files; patterns support `*` and `?`. A basename is accepted only when it resolves
to one allowed candidate from an explicit entry, a concrete pattern directory, or the fixed system
locations `/usr/local/bin`, `/usr/bin`, and `/bin` under `*`. Absolute paths remain available. Current program bytes are copied into sealed memory before launch. There is no
ambient PATH lookup, implicit shell evaluation, script/shebang support, caller environment, stdin,
detached mode, or network. Approved programs may launch helpers; approving an interpreter or shell
grants its normal argv authority.
Arguments are literal argv entries, limited to 128 entries and 64 KiB total.

Each run uses Bubblewrap namespaces and a delegated cgroup v2 leaf. The sandbox receives a minimal
runtime filesystem plus the requested working directory when it is included and does not intersect
an exclusion. Callers may mount it `read_only`; otherwise it is mounted read-write.
Stdout and stderr are
drained concurrently and capped at 1 MiB each. Timeout defaults to 30 seconds and is capped at 120
seconds. Completion, timeout, and output overflow kill the whole cgroup. Execution fails closed when
Bubblewrap or delegated cgroup v2 is unavailable.

Core emits a host-authored `process.execution.completed` effect with program path/hash, cwd,
outcome, duration, exit status, and byte counts—never argv or output. Tool arguments, returned output,
action results, and exact model logs remain durable. Do not use process tools for secrets.

## Web search

The `search` capability exposes action-only `habibi.search.search`. It is a narrow search adapter,
not generic HTTP. Configure `HABIBI_SEARCH_PROVIDER=brave` with `HABIBI_BRAVE_SEARCH_API_KEY`, or
`searxng` with one exact `HABIBI_SEARXNG_URL`. SearXNG accepts HTTPS or explicitly configured
loopback HTTP. Redirects are rejected; requests time out after 10 seconds; responses are capped at
1 MiB; result counts, titles, snippets, and URLs are bounded; only HTTP(S) citation URLs survive.
Provider credentials are injected by core and never enter Lua, browser code, events, or logs. An
unconfigured search host reports `configured() == false`; official Web Search then registers no model
tool until Habibi is reloaded with provider settings. SearXNG HTTP and engine failures are returned
as bounded, sanitized `provider_errors`; failures without results set `retryable = false`.

Search queries and normalized results do enter durable action/model history and are disclosed to the
configured provider. Snippets are untrusted third-party input and are not a license to republish page
content. Version 0.1 does not fetch result pages.

## JSON arrays

Empty Lua tables are ambiguous when converted to JSON. Use `habibi.array({})` when an empty
table must serialize as an array or deserialize into a Rust list.
