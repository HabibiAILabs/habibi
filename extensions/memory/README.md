# Memory

Adds relevant durable events to each model invocation through an ordinary Habibi context hook.

Memory contributes up to 20 events from the current event's causal chain, ordered oldest first, plus up to 20 locally embedded semantic matches. Duplicate events keep their causal placement. The extension formats the resulting context as JSON text; core only bounds and places that text in the invocation system message.

Semantic retrieval uses Habibi's explicitly installed pinned local embedding model. It performs no network access or model download.

## Capabilities

- `events`: reads durable events and performs bounded semantic event search.
- `context`: contributes formatted context text.

The extension registers no tools, routes, or event producers.
