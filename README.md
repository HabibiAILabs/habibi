# Habibi

Habibi is a local, event-sourced AI runtime built around one continuous conversation.
There are no core sessions: every incoming event joins one durable history, and each model
invocation receives context selected from that history.

Habibi's core provides:

- An append-only SQLite event store
- A native OpenAI ChatGPT/Codex OAuth model transport
- A multi-turn reaction and action-batch loop
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
| `HABIBI_CONTEXT_MESSAGES` | no | `40` |

## Run

```sh
mise exec -- cargo run
```

Then open `http://127.0.0.1:8787`. The home page presents Habibi itself; extension discovery
and enable/disable controls live at `/extensions`, and the complete durable event stream is
available at `/events`.

The included chat extension provides its web UI and APIs beneath
`/extensions/chat/`. It stores sessions and messages as `chat.*` events. UI preferences use
the extension's private KV namespace.

## Events API

`GET /api/events` returns the latest matching events in canonical sequence order. It defaults
to 100 events and supports `limit` (up to 1,000), `type`, `prefix`, `source`, `correlation_id`,
`before_sequence`, `after_sequence`, `from`, `to`, and preset `window` values (`15m`, `1h`,
`24h`, `7d`, `30d`, or `all`). Sequence cursors allow the UI to traverse all history.

Model invocation start events persist the exact JSON request sent to the model endpoint,
including `instructions`, converted `input` messages, tool definitions, reasoning settings, and
model. Completion events persist text, native output items, tool calls, and usage. Action batch,
proposal, execution, result, and continuation events are visible in the expanded event view.

Built-in tools can get/query events by metadata or payload text, create semantic links between
events, and traverse those links. The chat extension provides session lookup, keyword message
search, and user-visible message delivery tools.

## Chat API

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

See [`docs/extensions.md`](docs/extensions.md) for the extension contract.

## Test

```sh
mise exec -- cargo test
mise exec -- cargo clippy --all-targets -- -D warnings
```

## Core event types

- `runtime.started`
- `model.invocation.started`
- `model.invocation.completed`
- `model.invocation.failed`
- `action.batch.created`
- `action.proposed`
- `action.started`
- `action.succeeded`
- `action.failed`
- `action.batch.completed`
- `event.link.created`
- `event.link.removed`

The chat extension owns:

- `chat.session.created`
- `chat.session.renamed`
- `chat.session.archived`
- `chat.message.created`

SQLite's `events.sequence` is the canonical event order. Timestamps are informational.
