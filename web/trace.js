const form = document.querySelector("#trace-search");
const idInput = document.querySelector("#trace-id");
const latestButton = document.querySelector("#latest-trace");
const liveToggle = document.querySelector("#trace-live");
const status = document.querySelector("#trace-status");
const summary = document.querySelector("#trace-summary");
const map = document.querySelector("#trace-map");
const inspector = document.querySelector("#trace-inspector");
let trace = null;
let selectedKey = null;
let liveTimer = null;

const el = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};
const short = (value) => value ? `${String(value).slice(0, 8)}…${String(value).slice(-4)}` : "—";
const jsonBlock = (value) => {
  const pre = el("pre");
  pre.textContent = JSON.stringify(value, null, 2);
  return pre;
};

async function fetchJson(url) {
  const response = await fetch(url);
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
  return result;
}

async function openLatest() {
  const result = await fetchJson("/api/events?limit=1");
  const event = result.events.at(-1);
  if (!event) throw new Error("No events have been recorded yet.");
  await openTrace("event_id", event.id);
}

async function openId(value) {
  try {
    await openTrace("event_id", value);
  } catch (eventError) {
    try {
      await openTrace("correlation_id", value);
    } catch {
      throw eventError;
    }
  }
}

async function openTrace(kind, value, { preserveSelection = false } = {}) {
  status.textContent = "Loading causal trace…";
  const result = await fetchJson(`/api/trace?${kind}=${encodeURIComponent(value)}`);
  trace = result;
  const parameters = new URLSearchParams();
  parameters.set(kind, value);
  history.replaceState(null, "", `/trace?${parameters}`);
  idInput.value = value;
  if (!preserveSelection || !findSelected(selectedKey)) {
    selectedKey = result.focus_event_id ? `event:${result.focus_event_id}` : `event:${result.events.at(-1)?.record.id}`;
  }
  render();
}

function render() {
  renderSummary();
  const records = [
    ...trace.events.map(item => ({ kind: "event", item, record: item.record })),
    ...trace.logs.map(item => ({ kind: "log", item, record: item.record })),
  ].sort((left, right) => new Date(left.record.occurred_at) - new Date(right.record.occurred_at) || left.record.sequence - right.record.sequence);
  map.replaceChildren(...records.map(traceRow));
  const selected = findSelected(selectedKey);
  if (selected) renderInspector(selected.kind, selected.item);
  status.textContent = trace.truncated
    ? "Showing the newest bounded portion of this trace. Earlier records were truncated."
    : `${trace.events.length} events and ${trace.logs.length} processing records.`;
}

function renderSummary() {
  const models = trace.logs.filter(item => item.record.name === "model.invocation.started").length;
  const contexts = trace.logs.filter(item => item.record.name === "context.compiled").length;
  const tools = trace.events.filter(item => item.record.event_type === "action.requested").length;
  summary.hidden = false;
  summary.replaceChildren(
    summaryCard("Roots", trace.root_event_ids.length),
    summaryCard("Events", trace.events.length),
    summaryCard("Context builds", contexts),
    summaryCard("Model calls", models),
    summaryCard("Tool calls", tools),
  );
}

function summaryCard(label, value) {
  const card = el("div", "trace-summary-card");
  card.append(el("span", "", label), el("strong", "", String(value)));
  return card;
}

function traceRow(entry) {
  const row = el("div", `trace-row ${entry.kind}`);
  const key = `${entry.kind}:${entry.record.id}`;
  const button = el("button", `trace-node ${nodeClass(entry)}${selectedKey === key ? " selected" : ""}`);
  button.type = "button";
  button.dataset.key = key;
  const title = entry.kind === "event" ? entry.record.event_type : entry.record.name;
  const sequence = entry.kind === "event" ? `E${entry.record.sequence}` : `L${entry.record.sequence}`;
  const heading = el("span", "trace-node-heading");
  heading.append(el("span", "trace-sequence", sequence), el("strong", "", title));
  button.append(heading, el("span", "trace-node-meta", nodeMeta(entry)));
  button.addEventListener("click", () => {
    selectedKey = key;
    map.querySelectorAll(".trace-node.selected").forEach(node => node.classList.remove("selected"));
    button.classList.add("selected");
    renderInspector(entry.kind, entry.item);
  });
  const junction = el("div", "trace-junction");
  junction.append(el("span", "trace-dot"));
  if (entry.kind === "event") row.append(button, junction, el("div"));
  else row.append(el("div"), junction, button);
  return row;
}

function nodeClass(entry) {
  const name = entry.kind === "event" ? entry.record.event_type : entry.record.name;
  if (name === "context.compiled") return " context";
  if (name.startsWith("model.invocation.")) return " model";
  if (name.startsWith("action.")) return " tool";
  if (name.includes("failed")) return " failed";
  return "";
}

function nodeMeta(entry) {
  if (entry.kind === "event") {
    return `cause ${short(entry.record.causation_id)} · root ${short(entry.item.root_event_id)}`;
  }
  return [entry.record.category, `trigger ${short(entry.record.trigger_event_id)}`, formatDuration(entry.record.payload?.duration_ms)].filter(Boolean).join(" · ");
}

function formatDuration(value) {
  return value === undefined || value === null ? "" : `${Number(value).toLocaleString()} ms`;
}

function findSelected(key) {
  if (!key || !trace) return null;
  const [kind, id] = key.split(":");
  const items = kind === "event" ? trace.events : trace.logs;
  const item = items.find(candidate => candidate.record.id === id);
  return item ? { kind, item } : null;
}

function renderInspector(kind, item) {
  const record = item.record;
  inspector.replaceChildren();
  const heading = el("div", "trace-inspector-heading");
  heading.append(
    el("p", "eyebrow", kind === "event" ? "DOMAIN EVENT" : "OPERATIONAL LOG"),
    el("h2", "", kind === "event" ? record.event_type : record.name),
    el("time", "muted", new Date(record.occurred_at).toLocaleString()),
  );
  inspector.append(heading, identityGrid(kind, item));
  if (kind === "event") renderEventDetails(item);
  else renderLogDetails(item);
  inspector.append(disclosure("Complete record", record));
}

function identityGrid(kind, item) {
  const record = item.record;
  const grid = el("div", "trace-identity-grid");
  grid.append(identity("ID", record.id, () => selectRecord(kind, record.id)));
  grid.append(identity("Root", item.root_event_id, () => selectRecord("event", item.root_event_id)));
  if (kind === "event") {
    grid.append(identity("Cause", record.causation_id, () => selectRecord("event", record.causation_id)));
    grid.append(identity("Correlation", record.correlation_id));
  } else {
    grid.append(identity("Trigger", record.trigger_event_id, () => selectRecord("event", record.trigger_event_id)));
    grid.append(identity("Reaction", record.reaction_id));
  }
  return grid;
}

function identity(label, value, action) {
  const cell = el("div", "trace-identity");
  cell.append(el("span", "", label));
  if (value && action) {
    const button = el("button", "trace-id-link", String(value));
    button.type = "button";
    button.addEventListener("click", action);
    cell.append(button);
  } else cell.append(el("code", "", value || "—"));
  return cell;
}

function selectRecord(kind, id) {
  if (!id) return;
  const key = `${kind}:${id}`;
  const selected = findSelected(key);
  if (!selected) return;
  selectedKey = key;
  renderInspector(selected.kind, selected.item);
  const node = map.querySelector(`[data-key="${CSS.escape(key)}"]`);
  map.querySelectorAll(".trace-node.selected").forEach(item => item.classList.remove("selected"));
  node?.classList.add("selected");
  node?.scrollIntoView({ behavior: "smooth", block: "center" });
}

function renderEventDetails(item) {
  const record = item.record;
  if (record.event_type === "action.requested") {
    inspector.append(section("Tool input", {
      tool: record.payload.tool,
      arguments: record.payload.arguments,
      action_id: record.payload.action_id,
      tool_call_id: record.payload.tool_call_id,
    }));
    const results = trace.events.filter(candidate =>
      candidate.record.payload?.action_id === record.payload.action_id && candidate.record.event_type.startsWith("action.result."));
    for (const result of results) inspector.append(section("Tool result", result.record.payload));
  } else if (record.event_type.startsWith("action.result.")) {
    const request = trace.events.find(candidate =>
      candidate.record.event_type === "action.requested" && candidate.record.payload?.action_id === record.payload.action_id);
    if (request) inspector.append(section("Tool input", request.record.payload.arguments));
    inspector.append(section("Tool result", record.payload));
  } else {
    inspector.append(section("Event payload", record.payload));
  }
  if (item.caused_event_ids.length) {
    const children = el("section", "trace-detail-section");
    children.append(el("h3", "", "Caused events"));
    const links = el("div", "trace-chip-list");
    item.caused_event_ids.forEach(id => {
      const event = trace.events.find(candidate => candidate.record.id === id);
      const button = el("button", "trace-chip", event ? event.record.event_type : short(id));
      button.type = "button";
      button.addEventListener("click", () => selectRecord("event", id));
      links.append(button);
    });
    children.append(links);
    inspector.append(children);
  }
}

function renderLogDetails(item) {
  const record = item.record;
  if (record.name === "context.compiled") {
    const model = trace.logs.find(candidate => candidate.record.payload?.context_log_id === record.id && candidate.record.name === "model.invocation.started");
    inspector.append(section("Built context — exact model input", record.payload.input ?? model?.record.payload?.request?.input));
    inspector.append(metrics(record.payload, ["extension_hook_count", "extension_items", "rendered_bytes", "estimated_tokens", "hook_preparation_duration_ms", "rendering_duration_ms"]));
    return;
  }
  if (record.name === "model.invocation.started") {
    inspector.append(section("Model input", record.payload.request));
    const completion = trace.logs.find(candidate => candidate.record.payload?.started_log_id === record.id);
    if (completion) inspector.append(modelOutput(completion.record));
    return;
  }
  if (record.name === "model.invocation.completed" || record.name === "model.invocation.failed") {
    const started = trace.logs.find(candidate => candidate.record.id === record.payload?.started_log_id);
    if (started) inspector.append(section("Model input", started.record.payload.request));
    inspector.append(modelOutput(record));
    return;
  }
  inspector.append(section("Log payload", record.payload));
}

function modelOutput(record) {
  return section(record.name.endsWith("failed") ? "Model failure" : "Model output", record.name.endsWith("failed") ? record.payload : {
    content: record.payload.content,
    tool_calls: record.payload.tool_calls,
    output_items: record.payload.output_items,
    provider_response: record.payload.provider_response,
    usage: record.payload.usage,
    estimated_cost: record.payload.estimated_cost,
    duration_ms: record.payload.duration_ms,
  });
}

function metrics(payload, keys) {
  const grid = el("section", "trace-metrics");
  keys.forEach(key => {
    const cell = el("div");
    cell.append(el("span", "", key.replaceAll("_", " ")), el("strong", "", String(payload[key] ?? "—")));
    grid.append(cell);
  });
  return grid;
}

function section(title, value) {
  const sectionElement = el("section", "trace-detail-section");
  sectionElement.append(el("h3", "", title), jsonBlock(value));
  return sectionElement;
}

function disclosure(title, value) {
  const details = el("details", "trace-raw");
  const summaryElement = el("summary", "", title);
  details.append(summaryElement, jsonBlock(value));
  return details;
}

function showError(error) {
  status.textContent = error.message;
  status.classList.add("error");
}

form.addEventListener("submit", event => {
  event.preventDefault();
  status.classList.remove("error");
  const value = idInput.value.trim();
  if (value) openId(value).catch(showError);
});
latestButton.addEventListener("click", () => openLatest().catch(showError));
liveToggle.addEventListener("change", () => {
  clearInterval(liveTimer);
  liveTimer = liveToggle.checked ? setInterval(() => {
    if (trace) openTrace("correlation_id", trace.correlation_id, { preserveSelection: true }).catch(showError);
  }, 3000) : null;
});

const parameters = new URLSearchParams(location.search);
const eventId = parameters.get("event_id");
const correlationId = parameters.get("correlation_id");
if (eventId) openTrace("event_id", eventId).catch(showError);
else if (correlationId) openTrace("correlation_id", correlationId).catch(showError);
else openLatest().catch(showError);
