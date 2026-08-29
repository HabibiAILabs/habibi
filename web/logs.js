import { renderMarkdown } from "/assets/markdown.js";

const form = document.querySelector("#log-filters");
const list = document.querySelector("#log-list");
const count = document.querySelector("#log-count");
const range = document.querySelector("#log-range");
const loadOlder = document.querySelector("#load-older");
let displayed = [];
let oldestSequence = null;
let lastPageSize = 0;

function queryParameters(beforeSequence) {
  const parameters = new URLSearchParams();
  for (const [name, rawValue] of new FormData(form)) {
    const value = String(rawValue).trim();
    if (value) parameters.set(name, value);
  }
  if (beforeSequence) parameters.set("before_sequence", beforeSequence);
  return parameters;
}

async function queryLogs({ older = false } = {}) {
  const response = await fetch(`/api/logs?${queryParameters(older ? oldestSequence : null)}`);
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Query failed (${response.status})`);
  const page = result.logs.slice().reverse();
  lastPageSize = page.length;
  displayed = older ? [...displayed, ...page] : page;
  oldestSequence = displayed.at(-1)?.sequence || null;
  renderLogs();
}

function renderLogs() {
  count.textContent = `${displayed.length} log${displayed.length === 1 ? "" : "s"}`;
  range.textContent = displayed.length ? `#${displayed.at(-1).sequence} – #${displayed[0].sequence}` : "";
  list.replaceChildren(...displayed.map(logCard));
  loadOlder.hidden = lastPageSize < Number(new FormData(form).get("limit") || 100) || !oldestSequence;
  if (!displayed.length) {
    const message = document.createElement("p");
    message.className = "muted";
    message.textContent = "No logs match this query.";
    list.append(message);
  }
}

function logCard(log) {
  const article = document.createElement("article");
  article.className = `event-card ${log.level === "error" ? "model-event" : ""}`;

  const header = document.createElement("header");
  const identity = document.createElement("div");
  identity.className = "event-identity";
  const sequence = document.createElement("span");
  sequence.className = "event-sequence";
  sequence.textContent = `#${log.sequence}`;
  const name = document.createElement("strong");
  name.textContent = log.name;
  identity.append(sequence, name);
  const time = document.createElement("time");
  time.dateTime = log.occurred_at;
  time.textContent = new Date(log.occurred_at).toLocaleString();
  header.append(identity, time);

  const usage = log.payload?.usage;
  const estimatedCost = log.payload?.estimated_cost;
  const metadata = document.createElement("div");
  metadata.className = "event-metadata";
  appendMeta(metadata, "level", log.level);
  appendMeta(metadata, "category", log.category);
  appendMetaLink(metadata, "reaction", log.reaction_id, `/logs?reaction_id=${encodeURIComponent(log.reaction_id)}`);
  if (log.trigger_event_id) {
    appendMetaLink(metadata, "trigger", log.trigger_event_id, `/events?event_id=${encodeURIComponent(log.trigger_event_id)}`);
  }
  appendMeta(metadata, "event type", log.payload?.current_event_type);
  appendMeta(metadata, "batch", log.batch_id);
  appendMeta(metadata, "action", log.action_id);
  appendMeta(metadata, "tool call", log.tool_call_id);
  appendMeta(metadata, "tokens", usage?.total_tokens);
  appendMeta(metadata, "cached", usage?.cache_read);
  appendMeta(metadata, "cost", estimatedCost?.total_usd == null ? null : `$${estimatedCost.total_usd.toFixed(6)}`);

  const links = document.createElement("nav");
  links.className = "record-links";
  links.append(recordLink("Open trace", `/trace?correlation_id=${encodeURIComponent(log.correlation_id)}`));
  if (log.trigger_event_id) {
    links.append(recordLink("Trigger event", `/events?event_id=${encodeURIComponent(log.trigger_event_id)}`));
  }
  links.append(recordLink("Reaction logs", `/logs?reaction_id=${encodeURIComponent(log.reaction_id)}`));

  article.append(header, metadata, links);
  const structured = structuredModelView(log);
  if (structured) article.append(structured);
  article.append(rawDetails(log));
  return article;
}

function structuredModelView(log) {
  if (log.name === "model.invocation.started") return structuredRequest(log.payload);
  if (log.name === "model.invocation.completed") return structuredResponse(log.payload);
  if (log.name === "model.invocation.failed") return structuredFailure(log.payload);
  return null;
}

function structuredRequest(payload) {
  const request = payload.request || {};
  const section = modelSection("Structured model request");
  section.append(summaryGrid([
    ["Model", request.model || payload.model],
    ["Event type", payload.current_event_type],
    ["Input items", request.input?.length ?? 0],
    ["Advertised tools", request.tools?.length ?? 0],
  ]));

  if (request.instructions) {
    const details = disclosure("System instructions", true);
    details.append(renderMarkdown(request.instructions));
    section.append(details);
  }

  if (request.input?.length) {
    const details = disclosure(`Input · ${request.input.length} item${request.input.length === 1 ? "" : "s"}`);
    const items = document.createElement("div");
    items.className = "model-structured";
    request.input.forEach((item, index) => items.append(inputItem(item, index)));
    details.append(items);
    section.append(details);
  }

  if (request.tools?.length) {
    const details = disclosure(`Tools · ${request.tools.length}`);
    const tools = document.createElement("div");
    tools.className = "model-structured";
    for (const tool of request.tools) {
      const card = document.createElement("div");
      card.className = "model-tool-call";
      const title = document.createElement("strong");
      title.textContent = String(tool.name || "unknown").replaceAll("__", ".");
      card.append(title);
      if (tool.description) card.append(renderMarkdown(tool.description));
      const schema = disclosure("Input schema");
      schema.append(jsonBlock(tool.parameters));
      card.append(schema);
      tools.append(card);
    }
    details.append(tools);
    section.append(details);
  }
  return section;
}

function structuredResponse(payload) {
  const section = modelSection("Structured model response");
  const usage = payload.usage || {};
  section.append(summaryGrid([
    ["Model", payload.model || "—"],
    ["Event type", payload.current_event_type],
    ["Duration", formatDuration(payload.duration_ms)],
    ["Input tokens", usage.input],
    ["Cache read", usage.cache_read],
    ["Output tokens", usage.output],
    ["Total tokens", usage.total_tokens],
    ["Estimated cost", payload.estimated_cost?.total_usd == null ? "—" : `$${payload.estimated_cost.total_usd.toFixed(6)}`],
  ]));

  if (payload.content) {
    const output = document.createElement("section");
    const heading = document.createElement("h4");
    heading.textContent = "Assistant output";
    output.append(heading, renderMarkdown(payload.content));
    section.append(output);
  }

  if (payload.tool_calls?.length) {
    const calls = document.createElement("section");
    const heading = document.createElement("h4");
    heading.textContent = `Tool calls · ${payload.tool_calls.length}`;
    calls.append(heading);
    for (const call of payload.tool_calls) {
      const card = document.createElement("div");
      card.className = "model-tool-call";
      const title = document.createElement("strong");
      title.textContent = call.name || "unknown";
      card.append(title, jsonBlock(call.arguments));
      calls.append(card);
    }
    section.append(calls);
  }

  if (!payload.content && !payload.tool_calls?.length) {
    const message = document.createElement("p");
    message.className = "muted";
    message.textContent = "The model returned no text or tool calls.";
    section.append(message);
  }
  return section;
}

function structuredFailure(payload) {
  const section = modelSection("Model invocation failure");
  section.append(summaryGrid([
    ["Event type", payload.current_event_type],
    ["Duration", formatDuration(payload.duration_ms)],
  ]));
  const error = document.createElement("p");
  error.textContent = payload.error || "Unknown model error";
  section.append(error);
  return section;
}

function inputItem(item, index) {
  const card = document.createElement("div");
  card.className = "model-tool-call";
  const title = document.createElement("strong");
  title.textContent = `#${index + 1} · ${item.role || item.type || "input"}`;
  card.append(title);
  const text = inputText(item);
  if (text) {
    try {
      card.append(jsonBlock(JSON.parse(text)));
    } catch {
      card.append(renderMarkdown(text));
    }
  } else {
    card.append(jsonBlock(item));
  }
  return card;
}

function inputText(item) {
  if (!Array.isArray(item.content)) return null;
  return item.content
    .map((part) => part.text)
    .filter((text) => typeof text === "string")
    .join("\n");
}

function modelSection(title) {
  const section = document.createElement("section");
  section.className = "model-structured";
  const heading = document.createElement("h3");
  heading.textContent = title;
  section.append(heading);
  return section;
}

function summaryGrid(entries) {
  const grid = document.createElement("div");
  grid.className = "model-summary-grid";
  for (const [label, rawValue] of entries) {
    if (rawValue === null || rawValue === undefined) continue;
    const item = document.createElement("div");
    item.className = "model-summary-item";
    const name = document.createElement("span");
    name.textContent = label;
    const value = document.createElement("strong");
    value.textContent = String(rawValue);
    item.append(name, value);
    grid.append(item);
  }
  return grid;
}

function disclosure(label, open = false) {
  const details = document.createElement("details");
  details.open = open;
  const summary = document.createElement("summary");
  summary.textContent = label;
  details.append(summary);
  return details;
}

function rawDetails(log) {
  const details = disclosure(
    log.name === "model.invocation.started"
      ? "Raw model request log"
      : log.name === "model.invocation.completed"
        ? "Raw model response and usage"
        : "Raw log payload",
  );
  details.append(jsonBlock(log.payload));
  return details;
}

function jsonBlock(value) {
  const pre = document.createElement("pre");
  pre.textContent = JSON.stringify(value, null, 2);
  return pre;
}

function appendMeta(container, label, value) {
  if (value === null || value === undefined) return;
  const element = document.createElement("span");
  element.textContent = `${label}: ${value}`;
  element.title = String(value);
  container.append(element);
}

function appendMetaLink(container, label, value, href) {
  if (value === null || value === undefined) return;
  const link = document.createElement("a");
  link.href = href;
  link.textContent = `${label}: ${value}`;
  link.title = String(value);
  container.append(link);
}

function recordLink(label, href) {
  const link = document.createElement("a");
  link.href = href;
  link.textContent = label;
  return link;
}

function formatDuration(value) {
  return value == null ? "—" : `${Number(value).toLocaleString()} ms`;
}

function applyUrlFilters() {
  const parameters = new URLSearchParams(location.search);
  for (const [name, value] of parameters) {
    const field = form.elements.namedItem(name);
    if (field) field.value = value;
  }
}

function showError(error) {
  list.textContent = "";
  const message = document.createElement("p");
  message.className = "muted";
  message.textContent = error.message;
  list.append(message);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  queryLogs().catch(showError);
});
loadOlder.addEventListener("click", () => queryLogs({ older: true }).catch(showError));
document.querySelector("#model-logs").addEventListener("click", () => {
  form.reset();
  form.elements.name_prefix.value = "model.invocation.";
  queryLogs().catch(showError);
});
document.querySelector("#action-logs").addEventListener("click", () => {
  form.reset();
  form.elements.category.value = "action";
  queryLogs().catch(showError);
});

applyUrlFilters();
queryLogs().catch(showError);
