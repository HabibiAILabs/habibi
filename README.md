<p align="center">
  <img src="web/habibi-logo.svg" alt="Habibi" width="170">
</p>

<h1 align="center">Habibi</h1>

<p align="center">
  A local, event-sourced AI runtime with durable actions, searchable execution logs, and installable extensions.
</p>

<p align="center">
  <a href="https://github.com/HabibiAssistant/extensions">Official extensions</a>
  ·
  <a href="docs/extensions.md">Extension authoring</a>
  ·
  <a href="ROADMAP.md">Roadmap</a>
  ·
  <a href="SECURITY.md">Security</a>
  ·
  <a href="LICENSE">MIT license</a>
</p>

Habibi is a local, event-sourced AI runtime built around one continuous conversation.
There are no core sessions: every incoming event joins one durable history. Each model invocation
processes one current event; extensions may add their own event or message projections through
measured context hooks.

Habibi's core provides:

- An append-only SQLite event store
- A native OpenAI ChatGPT/Codex OAuth model transport
- A queue-driven event reactor with one model invocation per processed event
- A searchable, chain-scoped tool registry with measured suggestions, advertisements, calls, and outcomes
- A local Axum web server
- Capability-scoped Lua extensions
- Namespaced web routes and JSON KV storage for extensions

User-facing chat is the first extension. Its sessions organize chat events for the UI; they
do not isolate the model from Habibi's global event history.

## Requirements

- Rust (the repository's `mise.toml` installs the stable toolchain)
- An OpenAI account with a ChatGPT subscription that provides Codex access

Habibi does not depend on pi, an OpenAI API key, Node.js, or an external model proxy.

## Authenticate

Run Habibi's native device-code OAuth flow:

```sh
mise exec -- cargo run -- login
```

Habibi prints an OpenAI URL and one-time code. Credentials are stored at
`~/.config/habibi/auth.json` with user-only permissions on Unix and refreshed automatically.
Override the location with `HABIBI_AUTH_FILE`.

## Configure

```sh
cp .env.example .env
```

| Variable | Required | Default |
| --- | --- | --- |
| `HABIBI_MODEL` | no | `gpt-5.6-luna` |
| `HABIBI_THINKING` | no | provider default |
| `HABIBI_AUTH_FILE` | no | `~/.config/habibi/auth.json` |
| `HABIBI_OPENAI_CODEX_URL` | no | ChatGPT Codex Responses endpoint |
| `HABIBI_DB` | no | `habibi.db` |
| `HABIBI_BIND` | no | `127.0.0.1:8787` |
| `HABIBI_EXTENSIONS_DIR` | no | `extensions` |

## Run

```sh
mise exec -- cargo run
```

Then open `http://127.0.0.1:8787`. The home page presents Habibi itself; extension discovery
and enable/disable controls live at `/extensions`. The interactive causal view is at `/trace`;
domain history is at `/events`, operational execution at `/logs`, and usage totals at `/stats`.

Extensions are installed separately from the core runtime. Install the official packages you need,
then start Habibi:

```sh
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir chat
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir workspace
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir process
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir git
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir extension-studio
mise exec -- cargo run -- install https://github.com/HabibiAssistant/extensions.git --subdir web-search
mise exec -- cargo run
```

You can also install a local checkout:

```sh
mise exec -- cargo run -- install ../habibi-extensions --subdir chat
mise exec -- cargo run -- install ../habibi-extensions --subdir workspace
mise exec -- cargo run -- install ../habibi-extensions --subdir process
mise exec -- cargo run -- install ../habibi-extensions --subdir git
mise exec -- cargo run -- install ../habibi-extensions --subdir extension-studio
mise exec -- cargo run -- install ../habibi-extensions --subdir web-search
```

Installed extensions retain source, revision, semantic version, content hash, capabilities, and
installation time. Use `habibi update <id>` or the Extensions UI to check and apply updates, and
`habibi rollback <id>` to restore the previous installed generation. An extension must increase
its manifest version when its content changes.

Installed extensions are fully trusted local code. Their web applications and APIs share Habibi's
origin, and declared capabilities describe behavior for review rather than forming a hostile-code
security boundary. Every install and update is staged, automatically security/privacy scanned, and
Lua-validated before it can enter the active runtime; blocking findings abort the operation and
warnings remain visible in installation metadata and the Extensions UI. See [`SECURITY.md`](SECURITY.md)
for the scanner's coverage and limitations. Chat stores sessions and messages as `chat.*` events
and keeps UI preferences in its private KV namespace. Workspace remains denied until the user saves
one or more existing absolute root directories on the Extensions page.

## Events API

`GET /api/events` returns the latest matching events in canonical sequence order. It defaults
to 100 events and supports `limit` (up to 1,000), `type`, `prefix`, `source`, `correlation_id`,
`before_sequence`, `after_sequence`, `from`, `to`, and preset `window` values (`15m`, `1h`,
`24h`, `7d`, `30d`, or `all`). Sequence cursors allow the UI to traverse all history.

Action requests, structured results, batch barriers, tool effects, and semantic links are events.
Route-emitted domain events trigger model processing; internal action/effect/result facts are
represented to the model through the ordered `action.batch.completed` event rather than invoking
the model once for every bookkeeping event. Model requests/responses and execution diagnostics are
logs rather than reactor inputs.

`GET /api/logs` and `/logs` provide searchable operational history by level, category, name,
reaction, trigger, correlation, batch, action, tool call, time, payload text, and sequence. Model
logs include exact requests, native output items, parsed tool calls, token usage, cache reads and
writes when reported by the provider, and per-invocation cost estimates when pricing is configured.
The Logs UI adds structured request/response summaries, safe Markdown rendering for model text,
and direct links between logs and their trigger events while preserving the complete raw payload.
`GET /api/trace` accepts an `event_id` or `correlation_id` and returns a bounded joined trace with
each event's causal root and children plus correlated operational logs. `/trace` renders those two
streams as an interactive H-shaped path with paired tool inputs/results, exact model inputs/outputs,
and each compiled context. A `truncated` flag identifies chains beyond the 1,000-event or 2,000-log
view bounds.
`GET /api/stats` and `/stats` aggregate model usage globally and by model, plus tool advertisements,
distinct chains, calls, outcomes, schema-token estimates, and execution duration. Pricing comes from
`model-catalog.json`, selected by provider and model ID. The Stats page can refresh the catalog from
models.dev through `POST /api/models/refresh`; `GET /api/models` exposes the current catalog. Set
`HABIBI_MODEL_CATALOG` or `HABIBI_MODEL_CATALOG_URL` to use custom storage or a different source.
Every completed invocation stores its exact catalog entry and rates, so later refreshes never
rewrite historical estimates. See [`docs/model-catalog.md`](docs/model-catalog.md) for the format
and refresh semantics.

The always-advertised `habibi.tools.search` tool discovers matching built-in and extension tools.
Built-in tools can get/query events or logs, create semantic links between events, and traverse
those links. The chat extension suggests only its event-relevant reply tool; other chat tools remain
searchable through the registry.

## Chat API

The official chat extension is maintained at
[`HabibiAssistant/extensions`](https://github.com/HabibiAssistant/extensions/tree/main/chat).
Its web UI and API are mounted beneath `/extensions/chat/`.

```text
GET    /extensions/chat/api/sessions
POST   /extensions/chat/api/sessions
GET    /extensions/chat/api/sessions/:id
PATCH  /extensions/chat/api/sessions/:id
DELETE /extensions/chat/api/sessions/:id
GET    /extensions/chat/api/sessions/:id/messages
POST   /extensions/chat/api/sessions/:id/messages
GET    /extensions/chat/api/events
GET    /extensions/chat/api/preferences
PUT    /extensions/chat/api/preferences
```

## Workspace extension

The official Workspace extension provides searchable tools for bounded directory listing, UTF-8
reads, literal search, checked atomic writes and patches, one-level directory creation, same-root
moves, and nonrecursive deletion. Configure granted roots on `/extensions`; no grant means no
filesystem access. Tool search advertises Workspace definitions only when they are relevant.

## Process extension

The Linux-only Process extension runs exact user-granted native executables without implicit shell
evaluation. Each invocation verifies the executable hash, executes sealed bytes inside Bubblewrap namespaces, removes
network and ambient environment access, exposes one granted filesystem root, bounds output/time, and
kills the complete delegated cgroup afterward. Configure both root and executable grants on
`/extensions`. Arguments and returned output are durable; never use process tools for secrets.

## Git extension

Git `0.1` provides read-only status, diff, log, and show tools through Process. Each repository must
be an exact filesystem grant and is mounted read-only. Git hooks, fsmonitor, pagers, optional locks,
external diff/textconv, signatures, networking, and submodule traversal are disabled where relevant.

## Extension Studio

`/studio` provides a host-owned draft editor and review flow. Drafts use relative allowlisted text
paths, SHA-256 checked edits, the real package scanner and isolated runtime validation, and explicit
hash-bound installation. Model tools can create/edit/validate drafts but cannot install them.

## Web Search extension

Web Search supports Brave Search API or an explicitly configured self-hosted SearXNG origin. It
returns bounded source URLs and snippets without fetching pages. Queries leave Habibi and remain in
durable action/model history. Provider credentials stay host-side.

See [`docs/extensions.md`](docs/extensions.md) for the extension contract and host API guarantees.

## Test

```sh
mise exec -- cargo test
mise exec -- cargo clippy --all-targets -- -D warnings
```

## Core event types

- `action.requested`
- `action.result.succeeded`
- `action.result.failed`
- `action.batch.completed`
- `event.link.created`
- `event.link.removed`

The chat extension owns:

- `chat.session.created`
- `chat.session.started`
- `chat.session.renamed`
- `chat.session.archived`
- `chat.message.created`

The filesystem host owns:

- `workspace.file.created`
- `workspace.file.written`
- `workspace.file.patched`
- `workspace.file.deleted`
- `workspace.directory.created`
- `workspace.directory.deleted`
- `workspace.entry.moved`
- `process.execution.completed`

SQLite's `events.sequence` is canonical domain-event order. Logs have an independent operational
sequence. Timestamps are informational.
