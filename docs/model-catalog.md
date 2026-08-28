# Model catalog

Habibi resolves estimated pricing by `(provider, model ID)` from `model-catalog.json`. An entry contains input, output, cache-read, and optional cache-write prices in USD per million tokens.

```json
{
  "provider": "openai-codex",
  "id": "gpt-5.6-luna",
  "name": "GPT-5.6 Luna",
  "pricing": {
    "input_usd_per_million": 0.2,
    "output_usd_per_million": 1.2,
    "cache_read_usd_per_million": 0.02,
    "cache_write_usd_per_million": 0.25
  },
  "aliases": [],
  "source": "models.dev",
  "updated_at": "2026-08-28T00:00:00Z"
}
```

The catalog path defaults to `model-catalog.json` and can be changed with `HABIBI_MODEL_CATALOG`. The refresh source defaults to `https://models.dev/api.json` and can be changed with `HABIBI_MODEL_CATALOG_URL`.

- `GET /api/models` returns the active catalog.
- `POST /api/models/refresh` fetches and atomically persists current OpenAI prices.
- The Stats page exposes both operations.

Refresh merges remote entries with local entries. Exact `openai` model prices are also applied to matching `openai-codex` entries, while local aliases are retained.

Every completed invocation stores its resolved provider, model ID, price source, price timestamp, full rate snapshot, category subtotals, and total. Historical estimates therefore remain unchanged after catalog refreshes.

The bundled GPT-5.6 entries use the normal context tier. If Habibi later supports requests beyond that tier, context-tier selection must be represented explicitly in the catalog and invocation snapshot rather than silently changing a rate.
