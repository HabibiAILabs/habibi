# Habibi

Habibi is a local, event-sourced AI runtime built around one continuous conversation.
There are no sessions: each interaction is appended to a durable event log, and each
model invocation receives a context selected from that history.

This first implementation is intentionally small:

1. Read a user message from the terminal.
2. Append it to SQLite.
3. Load recent conversational events.
4. Invoke an OpenAI Codex model using native ChatGPT OAuth.
5. Persist the model invocation and assistant response.
6. Repeat.

There are no tools or extensions yet.

## Requirements

- Rust (the repository's `mise.toml` installs the stable toolchain)
- An OpenAI account with a ChatGPT subscription that provides Codex access

Habibi does not depend on pi, an OpenAI API key, Node.js, or an external model proxy.

## Authenticate

Run Habibi's native device-code OAuth flow:

```sh
mise exec -- cargo run -- login
```

Habibi prints an OpenAI URL and one-time code. Complete authorization in your browser.
The resulting access and refresh tokens are written to:

```text
~/.config/habibi/auth.json
```

The file is created with user-only permissions on Unix. Override its location with
`HABIBI_AUTH_FILE`. Access tokens are refreshed automatically before expiration.

## Configure

```sh
cp .env.example .env
```

Habibi recognizes:

| Variable | Required | Default |
| --- | --- | --- |
| `HABIBI_MODEL` | no | `gpt-5.4` |
| `HABIBI_THINKING` | no | provider default |
| `HABIBI_AUTH_FILE` | no | `~/.config/habibi/auth.json` |
| `HABIBI_OPENAI_CODEX_URL` | no | ChatGPT Codex Responses endpoint |
| `HABIBI_DB` | no | `habibi.db` |
| `HABIBI_CONTEXT_MESSAGES` | no | `40` |

## Run

```sh
mise exec -- cargo run
```

The conversation survives process restarts because its events remain in `habibi.db`.
Use `/quit` to leave the terminal loop.

## Test

```sh
mise exec -- cargo test
```

## Current event types

- `runtime.started`
- `user.message`
- `model.invocation.started`
- `model.invocation.completed`
- `model.invocation.failed`
- `assistant.message`

SQLite's `events.sequence` is the canonical event order. Event timestamps are
informational.
