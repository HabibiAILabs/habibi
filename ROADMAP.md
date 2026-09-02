# Habibi roadmap

Habibi is a local, event-sourced AI runtime. This roadmap describes direction rather than fixed release dates.

## Available now

- Durable SQLite domain events with causation and correlation
- Searchable operational logs separated from events
- Crash-recovered SQLite model inbox with one serial background worker
- Concurrent action groups with durable per-call ASAP/batch delivery
- Native OpenAI ChatGPT/Codex OAuth and local Ollama transports
- Model pricing catalog, token/cache accounting, and historical cost snapshots
- Capability-declared Lua extensions with web, KV, events, tools, and owned context hooks
- Semantic event-scoped tool surfaces with global advertisement, call, outcome, latency, and schema-token measurements
- Filtered, replayable SSE event tail with sequence cursors and live Chat/Events UIs
- Repeatable isolated deterministic/live eval harness with static reports
- Local and Git extension installation with source/revision/version/hash provenance
- Extension update checks, hot reload, rollback, and management UI
- Automatic extension security/privacy scanning before installation or update
- Official extensions distributed from `HabibiAILabs/extensions`
- User-managed filesystem root grants, capability-confined reads/search, checked atomic mutations, and host-authored effects
- Linux sandboxed process execution with exact executable grants, sealed images, Bubblewrap isolation, cgroup termination, and host-authored effects
- Read-only Git inspection through exact read-only repository sandboxes
- Bounded Brave/SearXNG web search with host-only credentials and citable normalized results

## Near term

### Extension lifecycle

- Append-only install, update, rollback, enable, and uninstall events
- Extension uninstall while preserving event and KV history by default
- Dirty-install detection and provenance verification
- Signed official catalog entries and optional signature verification
- Cached update checks and release-channel support
- Better scan reports with line numbers, reviewed exceptions, and capability diffs
- Package generation cleanup and crash recovery for interrupted installs

### Extension authoring

- Scoped draft read/write tools for Habibi
- Shared authoring guide exposed to the model
- Isolated extension test harness with fixture events and temporary KV storage
- User approval of generated diffs and capabilities before installation
- Install Habibi-authored drafts through the same package pipeline as Git/local extensions

### Engine recovery

- Persistent per-action receipts to reduce duplicate external effects after a process crash
- Replay and recovery tooling
- Trace export and semantic-link overlays for the causal visualization

## Medium term

### Official extension ecosystem

- Searchable official extension catalog
- Install from the core UI
- Per-extension release notes and update channels
- Compatibility ranges for Habibi API versions
- Extension health, validation, and provenance badges
- Community source support with trust and review indicators

### Runtime and model support

- Additional native model providers and OAuth transports beyond OpenAI and Ollama
- Provider/model selection policies by event type
- Context construction extensions and retrieval policies
- Budget and latency accounting across durable event dispatches
- Context-tier-aware pricing

### Local intelligence

- Embeddings and semantic event retrieval
- Durable entities and user-controlled memory projections
- Scheduled and background event sources
- Local notifications and approval requests

## Longer term

- Habibi can propose, implement, test, and maintain its own extensions
- Declarative extension blueprints for common integrations
- Multi-device event replication with explicit trust boundaries
- Encrypted backups and selective export/import
- Reproducible extension builds and stronger sandboxing options

## Principles

- Events are immutable domain facts; execution diagnostics are logs.
- Every submitted event is processed through an explicit dispatcher path.
- Actions and their results remain durable and attributable.
- Installed extensions are fully trusted local code; capabilities and scanning support informed review but do not pretend to make hostile code safe.
- Historical provenance and cost estimates retain the exact metadata used at the time.
- Local operation must not depend on Habibi-hosted cloud infrastructure.
