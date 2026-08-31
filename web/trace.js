import { createLiveBatch, createPermanentFailure, createRequestGate, describeGraphFailure, expireLiveIds, intersectEventIds, pruneLiveIds } from "/assets/graph-layout.mjs";
import { buildMemoryScene, pickMemoryNode } from "/assets/memory-graph-state.mjs";
import { createMemoryGraphRenderer } from "/assets/memory-graph.js";

const form = document.querySelector("#trace-search");
const idInput = document.querySelector("#trace-id");
const latestButton = document.querySelector("#latest-trace");
const liveToggle = document.querySelector("#trace-live");
const status = document.querySelector("#trace-status");
const summary = document.querySelector("#trace-summary");
const map = document.querySelector("#trace-map");
const inspector = document.querySelector("#trace-inspector");
const timelineTab = document.querySelector("#timeline-tab");
const graphTab = document.querySelector("#graph-tab");
const graphPanel = document.querySelector("#event-graph");
const graphForm = document.querySelector("#graph-filters");
const graphFilterPanel = document.querySelector("#graph-filter-panel");
const graphFilterSummary = document.querySelector("#graph-filter-summary");
const graphEventTypes = document.querySelector("#graph-event-types");
const graphType = document.querySelector("#graph-type");
const graphSource = document.querySelector("#graph-source");
const graphCorrelation = document.querySelector("#graph-correlation");
const graphLimit = document.querySelector("#graph-limit");
const graphStatus = document.querySelector("#graph-status");
const graphEmpty = document.querySelector("#graph-empty");
const graphCanvas = document.querySelector("#memory-graph-canvas");
const graphFatal = document.querySelector("#graph-fatal");
const graphRendererName = document.querySelector("#graph-renderer-name");
const graphEventList = document.querySelector("#graph-event-list");
const graphTooltip = document.querySelector("#graph-tooltip");
const graphLiveButton = document.querySelector("#graph-live");
const graphLiveStatus = document.querySelector("#graph-live-status");
const graphFocusButton = document.querySelector("#graph-focus");
let trace = null;
let graph = null;
let selectedKey = null;
let selectedGraphId = null;
let liveTimer = null;
let graphDrag = null;
const graphPointers = new Map();
let graphPinchDistance = 0;
let graphRenderer = null;
let graphRendererPromise = null;
let graphRendererError = null;
const graphFailure = createPermanentFailure(shutdownGraph);
let memoryScene = null;
let hoveredGraphId = null;
let graphFilters = null;
let graphEventSource = null;
let graphLiveEnabled = true;
const recentLiveIds = new Map();
const knownGraphTypes = new Set();
let graphLiveExpiryTimer = null;
const graphRequestGate = createRequestGate();
let graphAbortController = null;
const graphLiveBatch = createLiveBatch(ids => {
  if (!graphFailure.error && !graphPanel.hidden && graphLiveEnabled) openGraph({ liveIds: ids, activeFilters: graphFilters }).catch(handleGraphError);
});

const el = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};
const short = value => value ? `${String(value).slice(0, 8)}…${String(value).slice(-4)}` : "—";
const jsonBlock = value => {
  const pre = el("pre");
  pre.textContent = JSON.stringify(value, null, 2);
  return pre;
};

async function fetchJson(url, options) {
  const response = await fetch(url, options);
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
  const parameters = new URLSearchParams(location.search);
  parameters.delete("event_id");
  parameters.delete("correlation_id");
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
  if (selected && !map.hidden) renderInspector(selected.kind, selected.item);
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
  if (entry.kind === "event") return `cause ${short(entry.record.causation_id)} · root ${short(entry.item.root_event_id)}`;
  return [entry.record.category, `trigger ${short(entry.record.event_id)}`, formatDuration(entry.record.payload?.duration_ms)].filter(Boolean).join(" · ");
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
    grid.append(identity("Event", record.event_id, () => selectRecord("event", record.event_id)));
    grid.append(identity("Dispatch", record.dispatch_id));
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
  } else inspector.append(section("Event payload", record.payload));
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
  details.append(el("summary", "", title), jsonBlock(value));
  return details;
}

function setView(view, { load = true } = {}) {
  const graphActive = view === "graph";
  document.body.classList.toggle("graph-mode", graphActive);
  map.hidden = graphActive;
  graphPanel.hidden = !graphActive;
  timelineTab.classList.toggle("active", !graphActive);
  graphTab.classList.toggle("active", graphActive);
  timelineTab.setAttribute("aria-selected", String(!graphActive));
  graphTab.setAttribute("aria-selected", String(graphActive));
  timelineTab.tabIndex = graphActive ? -1 : 0;
  graphTab.tabIndex = graphActive ? 0 : -1;
  if (!graphFailure.error) graphRenderer?.setActive(graphActive);
  if (!homepageMode) {
    const parameters = new URLSearchParams(location.search);
    if (graphActive) parameters.set("view", "graph"); else parameters.delete("view");
    history.replaceState(null, "", `/trace?${parameters}`);
  }
  if (graphActive && load && !graphFailure.error) openGraph().catch(handleGraphError);
  else if (!graphActive) {
    cancelGraphRequest();
    closeGraphStream();
    clearLiveState();
    clearGraphHover();
    const selected = findSelected(selectedKey);
    if (selected) renderInspector(selected.kind, selected.item);
  }
}

function cancelGraphRequest() {
  graphRequestGate.invalidate();
  graphAbortController?.abort();
  graphAbortController = null;
}

function setGraphLiveStatus(text, className = "") {
  graphLiveStatus.textContent = text;
  graphLiveStatus.className = `graph-live-status ${className}`.trim();
}

function showGraphFatal(message) {
  graphFatal.hidden = false;
  graphFatal.textContent = describeGraphFailure(message);
  graphStatus.textContent = message;
  graphStatus.classList.add("error");
}

function shutdownGraph(error) {
  graphRendererError = error;
  cancelGraphRequest();
  closeGraphStream();
  clearTimeout(graphLiveExpiryTimer);
  graphLiveExpiryTimer = null;
  recentLiveIds.clear();
  graphLiveEnabled = false;
  graphLiveButton.textContent = "Live unavailable";
  graphLiveButton.setAttribute("aria-pressed", "false");
  setGraphLiveStatus("Live stopped", "paused");
  graphPanel.querySelectorAll(".graph-toolbar button, .graph-filters input, .graph-filters select, .graph-filters button").forEach(control => { control.disabled = true; });
  const renderer = graphRenderer;
  graphRenderer = null;
  try { renderer?.dispose(); } catch { /* The fatal state is already permanent; never retry a failed disposal. */ }
  graphRendererName.textContent = "vgpu/WebGPU stopped";
  showGraphFatal(error.message);
}

function latchGraphFatal(message) {
  const latched = graphFailure.latch(new Error(message));
  if (!latched.first) showGraphFatal(latched.error.message);
  return latched.error;
}

async function ensureGraphRenderer() {
  if (graphFailure.error) throw graphFailure.error;
  if (graphRenderer) return graphRenderer;
  if (graphRendererError) throw graphRendererError;
  if (!graphRendererPromise) {
    graphRendererPromise = createMemoryGraphRenderer(graphCanvas, {
      onFatal(message) {
        latchGraphFatal(message);
      },
    }).then(renderer => {
      if (graphFailure.error) {
        renderer.dispose();
        throw graphFailure.error;
      }
      graphRenderer = renderer;
      graphRenderer.setActive(!graphPanel.hidden);
      graphFatal.hidden = true;
      graphRendererName.textContent = renderer.renderer;
      return renderer;
    }).catch(error => {
      const failure = error instanceof Error ? error : new Error(String(error));
      throw latchGraphFatal(failure.message);
    });
  }
  return graphRendererPromise;
}

function closeGraphStream() {
  graphEventSource?.close();
  graphEventSource = null;
  graphLiveBatch.clear();
}

function removeLiveMarkers(ids) {
  ids.forEach(id => {
    const button = graphEventList.querySelector(`[data-id="${CSS.escape(id)}"]`);
    if (button) {
      button.classList.remove("live");
      button.textContent = button.dataset.label;
    }
  });
  updateMemoryScene();
}

function clearLiveState() {
  clearTimeout(graphLiveExpiryTimer);
  graphLiveExpiryTimer = null;
  const ids = [...recentLiveIds.keys()];
  recentLiveIds.clear();
  removeLiveMarkers(ids);
}

function scheduleLiveExpiry() {
  clearTimeout(graphLiveExpiryTimer);
  graphLiveExpiryTimer = null;
  if (graphFailure.error || !recentLiveIds.size) return;
  const delay = Math.max(0, Math.min(...recentLiveIds.values()) - Date.now());
  graphLiveExpiryTimer = setTimeout(() => {
    graphLiveExpiryTimer = null;
    removeLiveMarkers(expireLiveIds(recentLiveIds, Date.now()));
    scheduleLiveExpiry();
  }, delay);
}

function subscribeGraph() {
  closeGraphStream();
  if (graphFailure.error) return;
  if (!graphLiveEnabled || graphPanel.hidden || !graphFilters || !graph) {
    setGraphLiveStatus(graphLiveEnabled ? "Live unavailable" : "Live paused", "paused");
    return;
  }
  const query = new URLSearchParams({ after_sequence: String(graph.cursor) });
  if (graphFilters.eventType) query.set("exact_type", graphFilters.eventType);
  if (graphFilters.correlation) query.set("correlation_id", graphFilters.correlation);
  setGraphLiveStatus("Live connecting…", "reconnecting");
  const source = new EventSource(`/api/events/stream?${query}`);
  graphEventSource = source;
  source.addEventListener("open", () => { if (graphEventSource === source) setGraphLiveStatus("Live connected"); });
  source.addEventListener("error", () => { if (graphEventSource === source) setGraphLiveStatus("Live reconnecting…", "reconnecting"); });
  source.addEventListener("habibi.event", message => {
    if (graphEventSource !== source) return;
    const event = JSON.parse(message.data);
    if (graphFilters.eventType && event.event_type !== graphFilters.eventType) return;
    if (graphFilters.source && event.source !== graphFilters.source) return;
    if (graph.events.some(candidate => candidate.id === event.id)) return;
    graphLiveBatch.add(event.id);
  });
}

async function openGraph({ liveIds = [], activeFilters = null } = {}) {
  if (graphFailure.error) throw graphFailure.error;
  const filters = activeFilters || {
    eventType: graphType.value.trim(),
    source: graphSource.value.trim(),
    correlation: graphCorrelation.value.trim(),
    limit: graphLimit.value,
  };
  closeGraphStream();
  const generation = graphRequestGate.next();
  graphAbortController?.abort();
  const controller = new AbortController();
  graphAbortController = controller;
  graphStatus.classList.remove("error");
  graphStatus.textContent = "Loading memories…";
  const query = new URLSearchParams({ limit: filters.limit });
  if (filters.eventType) query.set("type", filters.eventType);
  if (filters.source) query.set("source", filters.source);
  if (filters.correlation) query.set("correlation_id", filters.correlation);
  try {
    const result = await fetchJson(`/api/event-graph?${query}`, { signal: controller.signal });
    if (!graphRequestGate.isCurrent(generation)) return;
    graph = result;
    graphFilters = filters;
    result.events.forEach(event => knownGraphTypes.add(event.event_type));
    renderGraphFilterUi(filters);
    if (!homepageMode) {
      const parameters = new URLSearchParams(location.search);
      for (const name of ["graph_type", "graph_source", "graph_correlation", "graph_limit"]) parameters.delete(name);
      if (filters.eventType) parameters.set("graph_type", filters.eventType);
      if (filters.source) parameters.set("graph_source", filters.source);
      if (filters.correlation) parameters.set("graph_correlation", filters.correlation);
      parameters.set("graph_limit", filters.limit);
      history.replaceState(null, "", `/trace?${parameters}`);
    }
    if (!graph.events.some(event => event.id === selectedGraphId)) selectedGraphId = null;
    pruneLiveIds(recentLiveIds, result.events);
    intersectEventIds(liveIds, result.events).forEach(id => recentLiveIds.set(id, Date.now() + 6000));
    scheduleLiveExpiry();
    renderGraph();
    await ensureGraphRenderer();
    updateMemoryScene();
    subscribeGraph();
  } catch (error) {
    if (!graphRequestGate.isCurrent(generation) || error.name === "AbortError") return;
    if (graph) subscribeGraph();
    throw error;
  } finally {
    if (graphRequestGate.isCurrent(generation)) graphAbortController = null;
  }
}

function renderGraph() {
  graphEventList.replaceChildren();
  graphEmpty.hidden = graph.events.length > 0;
  if (!graph.events.length) {
    selectedGraphId = null;
    hoveredGraphId = null;
    graphFocusButton.disabled = true;
    graphTooltip.hidden = true;
    clearLiveState();
    graphStatus.textContent = "0 memories.";
    renderEmptyInspector("No memory is available to inspect.");
    return;
  }
  expireLiveIds(recentLiveIds, Date.now());
  graph.events.forEach(event => graphEventList.append(accessibleGraphNode(event)));
  const selected = graph.events.find(event => event.id === selectedGraphId);
  graphFocusButton.disabled = !selected;
  if (selected) renderGraphInspector(selected); else renderEmptyInspector("Select a memory to inspect its causal family and correlation.");
  renderGraphStatus(0);
}

function renderGraphStatus(boundaryCausal) {
  const visibleLinks = graph.links.filter(link => graph.events.some(event => event.id === link.from_event_id) && graph.events.some(event => event.id === link.to_event_id)).length;
  const notices = [];
  if (boundaryCausal) notices.push(`${boundaryCausal} missing causal parent ${boundaryCausal === 1 ? "boundary" : "boundaries"}`);
  if (graph.events_truncated) notices.push("older memories omitted by the node limit");
  if (graph.links_truncated) notices.push("semantic links capped at 2,000");
  graphStatus.textContent = `${graph.events.length} memories · ${visibleLinks} visible semantic links${notices.length ? ` · ${notices.join(" · ")}` : ""}.`;
}

function updateMemoryScene() {
  if (graphFailure.error || !graph || !graphRenderer) return;
  memoryScene = buildMemoryScene(graph.events, graph.links, {
    selectedId: selectedGraphId,
    hoveredId: hoveredGraphId,
    liveIds: recentLiveIds,
  });
  graphRenderer.setScene(memoryScene);
  graphEventList.querySelector(".graph-boundary-note")?.remove();
  if (memoryScene.boundaryCausal) {
    const note = el("div", "graph-boundary-note", `${memoryScene.boundaryCausal} ${memoryScene.boundaryCausal === 1 ? "memory has a parent" : "memories have parents"} outside this window; dashed amber boundary markers show those missing parents.`);
    note.setAttribute("role", "listitem");
    graphEventList.append(note);
  }
  renderGraphStatus(memoryScene.boundaryCausal);
  graphEventList.querySelectorAll("button[data-id]").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.id === selectedGraphId)));
}

function renderGraphFilterUi(filters) {
  const active = [filters.eventType && `type ${filters.eventType}`, filters.source && `source ${filters.source}`, filters.correlation && `correlation ${short(filters.correlation)}`].filter(Boolean);
  graphFilterSummary.textContent = active.length ? active.join(" · ") : "All live memories";
  graphEventTypes.replaceChildren(...[...knownGraphTypes].sort().map(type => {
    const option = document.createElement("option");
    option.value = type;
    return option;
  }));
}

function accessibleGraphNode(event) {
  const item = el("div");
  item.setAttribute("role", "listitem");
  const live = recentLiveIds.has(event.id);
  const label = `E${event.sequence} · ${event.event_type} · correlation ${short(event.correlation_id)}`;
  const button = el("button", live ? "live" : "", `${live ? "New memory · " : ""}${label}`);
  button.type = "button";
  button.dataset.id = event.id;
  button.dataset.label = label;
  button.setAttribute("aria-pressed", String(event.id === selectedGraphId));
  button.addEventListener("click", () => { selectGraphEvent(event.id); focusGraphEvent(event.id); });
  item.append(button);
  return item;
}

function selectGraphEvent(id) {
  selectedGraphId = id;
  graphFocusButton.disabled = false;
  updateMemoryScene();
  const event = graph.events.find(candidate => candidate.id === id);
  if (event) renderGraphInspector(event);
}

function renderEmptyInspector(message) {
  const empty = el("div", "trace-empty");
  const logo = document.createElement("img");
  logo.src = "/assets/habibi-logo.svg";
  logo.alt = "";
  empty.append(logo, el("p", "", message));
  inspector.replaceChildren(empty);
}

function renderGraphInspector(event) {
  const scene = buildMemoryScene(graph.events, graph.links, { selectedId: event.id, liveIds: recentLiveIds });
  inspector.replaceChildren();
  const heading = el("div", "trace-inspector-heading");
  heading.append(el("p", "eyebrow", "DURABLE MEMORY"), el("h2", "", event.event_type), el("time", "muted", new Date(event.occurred_at).toLocaleString()));
  const grid = el("div", "trace-identity-grid");
  grid.append(identity("ID", event.id), identity("Sequence", String(event.sequence)), identity("Cause", event.causation_id), identity("Correlation", event.correlation_id));
  const relationships = graph.links.filter(link => link.from_event_id === event.id || link.to_event_id === event.id);
  const memoryStats = {
    visible_ancestors: scene.family.ancestors.size,
    visible_descendants: scene.family.descendants.size,
    omitted_parent_boundary: scene.family.omittedAncestors,
    correlated_memories: graph.events.filter(candidate => candidate.correlation_id === event.correlation_id).length,
  };
  const open = el("button", "primary", "Open causal trace");
  open.type = "button";
  open.addEventListener("click", () => openGraphTrace(event.id));
  inspector.append(heading, grid, open, section("Visible memory family", memoryStats), section("Event payload", event.payload));
  if (relationships.length) inspector.append(section("Semantic relationships", relationships));
  inspector.append(disclosure("Complete record", event));
}

function openGraphTrace(id) {
  if (homepageMode) {
    location.assign(`/trace?event_id=${encodeURIComponent(id)}`);
    return;
  }
  setView("timeline", { load: false });
  openTrace("event_id", id).catch(showError);
}

function graphIdAtClientPoint(clientX, clientY, radius = 20) {
  if (graphFailure.error || !memoryScene || !graphRenderer) return null;
  const bounds = graphCanvas.getBoundingClientRect();
  const scaleX = bounds.width / graphCanvas.width;
  const scaleY = bounds.height / graphCanvas.height;
  const candidates = memoryScene.nodes.map(node => {
    const point = graphRenderer.project(node.position);
    return {
      id: node.id,
      x: bounds.left + point.x * scaleX,
      y: bounds.top + point.y * scaleY,
      depth: point.depth,
      radius: node.radius * Math.max(scaleX, scaleY),
      visible: node.pickable,
    };
  });
  return pickMemoryNode(candidates, clientX, clientY, radius);
}

function showGraphHover(clientX, clientY) {
  const id = graphIdAtClientPoint(clientX, clientY, 18);
  if (hoveredGraphId !== id) { hoveredGraphId = id; updateMemoryScene(); }
  if (!id) { graphTooltip.hidden = true; return; }
  const event = graph.events.find(candidate => candidate.id === id);
  if (!event) return;
  graphTooltip.textContent = `${event.event_type}\n${event.source} · E${event.sequence}\nCorrelation ${event.correlation_id}`;
  const stage = graphTooltip.parentElement;
  const bounds = stage.getBoundingClientRect();
  graphTooltip.hidden = false;
  const maximumLeft = Math.max(8, stage.clientWidth - graphTooltip.offsetWidth - 8);
  const maximumTop = Math.max(8, stage.clientHeight - graphTooltip.offsetHeight - 8);
  graphTooltip.style.left = `${Math.min(maximumLeft, Math.max(8, clientX - bounds.left + 12))}px`;
  graphTooltip.style.top = `${Math.min(maximumTop, Math.max(8, clientY - bounds.top + 12))}px`;
}

function clearGraphHover() {
  if (!hoveredGraphId && graphTooltip.hidden) return;
  hoveredGraphId = null;
  graphTooltip.hidden = true;
  updateMemoryScene();
}

function focusGraphEvent(id) {
  if (graphFailure.error) return;
  const node = memoryScene?.nodes.find(candidate => candidate.id === id);
  if (node) graphRenderer?.focus(node.position);
}

function fitGraph() { if (!graphFailure.error) graphRenderer?.fit(); }
function zoomGraph(factor) { if (!graphFailure.error) graphRenderer?.zoom(factor); }

function showError(error) {
  status.textContent = error.message;
  status.classList.add("error");
}

function handleGraphError(error) {
  if (error.name === "AbortError") return;
  const message = error instanceof Error ? error.message : String(error);
  if (graphFailure.error || message.includes("WebGPU") || message.includes("renderer is disposed")) {
    latchGraphFatal(graphFailure.error?.message || message);
    return;
  }
  graphStatus.textContent = message;
  graphStatus.classList.add("error");
}

form.addEventListener("submit", event => { event.preventDefault(); status.classList.remove("error"); const value = idInput.value.trim(); if (value) openId(value).catch(showError); });
latestButton.addEventListener("click", () => openLatest().catch(showError));
liveToggle.addEventListener("change", () => {
  clearInterval(liveTimer);
  liveTimer = liveToggle.checked ? setInterval(() => { if (trace) openTrace("correlation_id", trace.correlation_id, { preserveSelection: true }).catch(showError); }, 3000) : null;
});
timelineTab.addEventListener("click", () => setView("timeline"));
graphTab.addEventListener("click", () => setView("graph"));
[timelineTab, graphTab].forEach((tab, index, tabs) => tab.addEventListener("keydown", event => {
  let nextIndex = null;
  if (event.key === "ArrowRight") nextIndex = (index + 1) % tabs.length;
  else if (event.key === "ArrowLeft") nextIndex = (index - 1 + tabs.length) % tabs.length;
  else if (event.key === "Home") nextIndex = 0;
  else if (event.key === "End") nextIndex = tabs.length - 1;
  if (nextIndex === null) return;
  event.preventDefault();
  const nextTab = tabs[nextIndex];
  nextTab.focus();
  setView(nextTab === graphTab ? "graph" : "timeline");
}));
graphForm.addEventListener("submit", event => { event.preventDefault(); openGraph().catch(handleGraphError); });
document.querySelector("#graph-reset-filters").addEventListener("click", () => { graphType.value = ""; graphSource.value = ""; graphCorrelation.value = ""; openGraph().catch(handleGraphError); });
document.querySelector("#graph-zoom-in").addEventListener("click", () => zoomGraph(1.2));
document.querySelector("#graph-zoom-out").addEventListener("click", () => zoomGraph(0.82));
document.querySelector("#graph-fit").addEventListener("click", fitGraph);
graphFocusButton.addEventListener("click", () => { if (selectedGraphId) focusGraphEvent(selectedGraphId); });
graphLiveButton.addEventListener("click", () => {
  graphLiveEnabled = !graphLiveEnabled;
  graphLiveButton.textContent = graphLiveEnabled ? "Pause live" : "Resume live";
  graphLiveButton.setAttribute("aria-pressed", String(graphLiveEnabled));
  if (graphLiveEnabled) subscribeGraph(); else { closeGraphStream(); setGraphLiveStatus("Live paused", "paused"); }
});
document.querySelector("#graph-clear").addEventListener("click", () => {
  selectedGraphId = null;
  graphFocusButton.disabled = true;
  updateMemoryScene();
  renderEmptyInspector("Select a memory to inspect its causal family and correlation.");
});
graphCanvas.addEventListener("wheel", event => { event.preventDefault(); zoomGraph(event.deltaY < 0 ? 1.12 : 0.89); }, { passive: false });
graphCanvas.addEventListener("pointerdown", event => {
  if (event.pointerType === "mouse" && event.button !== 0) return;
  clearGraphHover();
  graphPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  graphDrag = graphPointers.size === 1 ? { x: event.clientX, y: event.clientY, lastX: event.clientX, lastY: event.clientY } : null;
  graphCanvas.setPointerCapture(event.pointerId);
  graphCanvas.classList.add("dragging");
});
graphCanvas.addEventListener("pointermove", event => {
  if (graphPointers.has(event.pointerId)) graphPointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  if (graphPointers.size === 2) {
    const [left, right] = [...graphPointers.values()];
    const distance = Math.hypot(left.x - right.x, left.y - right.y);
    if (graphPinchDistance) graphRenderer?.zoom(distance / graphPinchDistance);
    graphPinchDistance = distance;
    graphDrag = null;
    return;
  }
  if (!graphDrag) { showGraphHover(event.clientX, event.clientY); return; }
  graphRenderer?.rotate(event.clientX - graphDrag.lastX, event.clientY - graphDrag.lastY);
  graphDrag.lastX = event.clientX;
  graphDrag.lastY = event.clientY;
});
graphCanvas.addEventListener("pointerup", event => {
  const drag = graphDrag;
  graphPointers.delete(event.pointerId);
  if (graphPointers.size < 2) graphPinchDistance = 0;
  graphDrag = null;
  graphCanvas.releasePointerCapture(event.pointerId);
  graphCanvas.classList.remove("dragging");
  if (!drag || Math.hypot(event.clientX - drag.x, event.clientY - drag.y) > 4) return;
  const id = graphIdAtClientPoint(event.clientX, event.clientY, 24);
  if (id) selectGraphEvent(id);
});
graphCanvas.addEventListener("pointercancel", event => { graphPointers.delete(event.pointerId); graphPinchDistance = 0; graphDrag = null; graphCanvas.classList.remove("dragging"); });
graphCanvas.addEventListener("pointerleave", () => { if (!graphDrag) clearGraphHover(); });
graphCanvas.addEventListener("dblclick", event => { const id = graphIdAtClientPoint(event.clientX, event.clientY, 24); if (id) openGraphTrace(id); });

const parameters = new URLSearchParams(location.search);
const homepageMode = location.pathname === "/";
const graphMode = homepageMode || parameters.get("view") === "graph";
graphType.value = parameters.get("graph_type") || "";
graphSource.value = parameters.get("graph_source") || "";
graphCorrelation.value = parameters.get("graph_correlation") || "";
if (["100", "250", "500", "1000"].includes(parameters.get("graph_limit"))) graphLimit.value = parameters.get("graph_limit");
if (graphType.value || graphSource.value || graphCorrelation.value || parameters.has("graph_limit")) graphFilterPanel.open = true;
const eventId = parameters.get("event_id");
const correlationId = parameters.get("correlation_id");
if (!homepageMode) {
  if (eventId) openTrace("event_id", eventId).catch(showError);
  else if (correlationId) openTrace("correlation_id", correlationId).catch(showError);
  else openLatest().catch(showError);
}
if (graphMode) setView("graph");
window.addEventListener("beforeunload", () => { closeGraphStream(); graphRenderer?.dispose(); });
