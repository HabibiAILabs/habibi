import { renderMarkdown } from "/assets/markdown.js";

const MARKDOWN_KEYS = new Set(["content", "description", "error", "instructions", "message", "preview", "query_text", "text"]);

export function humanizeKey(key) {
  return String(key).replaceAll("_", " ").replace(/\b\w/g, letter => letter.toUpperCase());
}

export function renderToolSurface(payload) {
  const section = document.createElement("section");
  section.className = "tool-surface-view";
  section.append(renderRecord({
    current_event_id: payload.current_event_id,
    current_event_type: payload.current_event_type,
    query_text: payload.query_text,
    catalog_generation: payload.catalog_generation,
    advertised_tools: payload.advertised,
    final_limit: payload.final_limit,
    minimum_similarity: payload.minimum_similarity,
    semantic_candidate_limit: payload.semantic_limit,
    previously_used_limit: payload.used_limit,
    schema_bytes: payload.advertised_schema_bytes,
    estimated_schema_tokens: payload.estimated_advertised_schema_tokens,
    embedding_model: payload.embedding_model,
    embedding_revision: payload.embedding_revision,
    duration_ms: payload.duration_ms,
  }));

  const explanation = document.createElement("p");
  explanation.className = "muted";
  explanation.textContent = "Previously used tools in this correlation are selected first. Semantic matches above the minimum similarity fill the remaining slots, sorted by score. ‘Both’ means both rules selected the tool.";
  section.append(explanation);

  const tools = document.createElement("div");
  tools.className = "tool-surface-tools";
  for (const tool of payload.tools || []) {
    const card = document.createElement("article");
    card.className = "model-tool-call";
    const title = document.createElement("strong");
    title.textContent = tool.tool || "Unknown tool";
    card.append(title, renderRecord({
      reason: tool.reason,
      semantic_rank: tool.rank,
      similarity: tool.score == null ? undefined : Number(tool.score.toFixed(4)),
      schema_bytes: tool.schema_bytes,
      estimated_schema_tokens: tool.estimated_schema_tokens,
    }));
    tools.append(card);
  }
  section.append(tools);
  return section;
}

export function renderRecord(value, { omitKeys = [] } = {}) {
  const omitted = new Set(omitKeys);
  return renderValue(value, null, omitted);
}

function renderValue(value, key, omitted) {
  if (value === null || value === undefined) return text("—", "record-empty");
  if (Array.isArray(value)) {
    if (!value.length) return text("None", "record-empty");
    const list = document.createElement("ol");
    list.className = "record-array";
    for (const item of value) {
      const row = document.createElement("li");
      row.append(renderValue(item, key, omitted));
      list.append(row);
    }
    return list;
  }
  if (typeof value === "object") {
    const fields = document.createElement("dl");
    fields.className = "record-fields";
    const entries = Object.entries(value).filter(([name, item]) => !omitted.has(name) && item !== undefined);
    if (!entries.length) return text("None", "record-empty");
    for (const [name, item] of entries) {
      const term = document.createElement("dt");
      term.textContent = humanizeKey(name);
      const detail = document.createElement("dd");
      detail.append(renderValue(item, name, omitted));
      fields.append(term, detail);
    }
    return fields;
  }
  if (typeof value === "boolean") return text(value ? "Yes" : "No");
  if (typeof value === "number") return text(value.toLocaleString());
  const string = String(value);
  if (MARKDOWN_KEYS.has(key) && (string.includes("\n") || string.length > 100)) {
    const rendered = renderMarkdown(string);
    rendered.classList.add("record-markdown");
    return rendered;
  }
  return text(string, looksLikeIdentifier(string) ? "record-code" : "");
}

function text(value, className = "") {
  const element = document.createElement("span");
  if (className) element.className = className;
  element.textContent = value;
  return element;
}

function looksLikeIdentifier(value) {
  return /^([0-9a-f]{8}-|01[a-z0-9]{6,}|sha256:|[a-f0-9]{32,}$)/i.test(value);
}
