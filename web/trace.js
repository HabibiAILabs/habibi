import { compensateAnchor, createLiveBatch, createRequestGate, expireLiveIds, fitTransform, intersectEventIds, layoutEvents, nearestNode, pruneLiveIds, trimLine } from "/assets/graph-layout.mjs";

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
const graphType = document.querySelector("#graph-type");
const graphSource = document.querySelector("#graph-source");
const graphCorrelation = document.querySelector("#graph-correlation");
const graphLimit = document.querySelector("#graph-limit");
const graphStatus = document.querySelector("#graph-status");
const graphEmpty = document.querySelector("#graph-empty");
const graphSvg = document.querySelector("#graph-svg");
const graphViewport = document.querySelector("#graph-viewport");
const graphEventList = document.querySelector("#graph-event-list");
const graphTooltip = document.querySelector("#graph-tooltip");
const graphLiveButton = document.querySelector("#graph-live");
const graphLiveStatus = document.querySelector("#graph-live-status");
const graphFocusButton = document.querySelector("#graph-focus");
const svgNamespace = "http://www.w3.org/2000/svg";
let trace = null;
let graph = null;
let selectedKey = null;
let selectedGraphId = null;
let liveTimer = null;
let graphWorld = { width: 1200, height: 620 };
let graphPositions = new Map();
let graphTransform = { x: 0, y: 0, scale: 1 };
let graphDrag = null;
let hoveredGraphId = null;
let graphFilters = null;
let graphEventSource = null;
let graphLiveEnabled = true;
let graphInitialFit = true;
const recentLiveIds = new Map();
let graphLiveExpiryTimer = null;
const graphRequestGate = createRequestGate();
let graphAbortController = null;
let graphResizeTimer = null;
const graphLiveBatch = createLiveBatch(ids => {
  if (!graphPanel.hidden && graphLiveEnabled) openGraph({ preserveViewport: true, liveIds: ids, activeFilters: graphFilters }).catch(handleGraphError);
});

const el = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};
const svgEl = (tag, attributes = {}) => {
  const node = document.createElementNS(svgNamespace, tag);
  Object.entries(attributes).forEach(([name, value]) => node.setAttribute(name, String(value)));
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
  map.hidden = graphActive;
  graphPanel.hidden = !graphActive;
  timelineTab.classList.toggle("active", !graphActive);
  graphTab.classList.toggle("active", graphActive);
  timelineTab.setAttribute("aria-selected", String(!graphActive));
  graphTab.setAttribute("aria-selected", String(graphActive));
  timelineTab.tabIndex = graphActive ? -1 : 0;
  graphTab.tabIndex = graphActive ? 0 : -1;
  const parameters = new URLSearchParams(location.search);
  if (graphActive) parameters.set("view", "graph");
  else parameters.delete("view");
  history.replaceState(null, "", `/trace?${parameters}`);
  if (graphActive && load) {
    graphInitialFit = true;
    openGraph().catch(handleGraphError);
  } else if (!graphActive) {
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

function closeGraphStream() {
  graphEventSource?.close();
  graphEventSource = null;
  graphLiveBatch.clear();
}

function removeLiveMarkers(ids) {
  ids.forEach(id => {
    graphViewport.querySelector(`[data-id="${CSS.escape(id)}"]`)?.classList.remove("live");
    const button = graphEventList.querySelector(`[data-id="${CSS.escape(id)}"]`);
    if (button) {
      button.classList.remove("live");
      button.textContent = button.dataset.label;
    }
  });
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
  if (!recentLiveIds.size) return;
  const delay = Math.max(0, Math.min(...recentLiveIds.values()) - Date.now());
  graphLiveExpiryTimer = setTimeout(() => {
    graphLiveExpiryTimer = null;
    removeLiveMarkers(expireLiveIds(recentLiveIds, Date.now()));
    scheduleLiveExpiry();
  }, delay);
}

function subscribeGraph() {
  closeGraphStream();
  if (!graphLiveEnabled || graphPanel.hidden || !graphFilters || !("EventSource" in globalThis)) {
    setGraphLiveStatus(graphLiveEnabled ? "Live unavailable" : "Live paused", "paused");
    return;
  }
  const query = new URLSearchParams();
  query.set("after_sequence", String(graph.cursor));
  if (graphFilters.eventType) query.set("exact_type", graphFilters.eventType);
  if (graphFilters.correlation) query.set("correlation_id", graphFilters.correlation);
  setGraphLiveStatus("Live connecting…", "reconnecting");
  const source = new EventSource(`/api/events/stream?${query}`);
  graphEventSource = source;
  source.addEventListener("open", () => {
    if (graphEventSource === source) setGraphLiveStatus("Live connected");
  });
  source.addEventListener("error", () => {
    if (graphEventSource === source) setGraphLiveStatus("Live reconnecting…", "reconnecting");
  });
  source.addEventListener("habibi.event", message => {
    if (graphEventSource !== source) return;
    const event = JSON.parse(message.data);
    if (graphFilters.eventType && event.event_type !== graphFilters.eventType) return;
    if (graphFilters.source && event.source !== graphFilters.source) return;
    if (graph.events.some(candidate => candidate.id === event.id)) return;
    graphLiveBatch.add(event.id);
  });
}

async function openGraph({ preserveViewport = false, liveIds = [], activeFilters = null } = {}) {
  const filters = activeFilters || {
    eventType: graphType.value.trim(),
    source: graphSource.value.trim(),
    correlation: graphCorrelation.value.trim(),
    limit: graphLimit.value,
  };
  const viewportAnchor = preserveViewport ? captureGraphAnchor() : null;
  closeGraphStream();
  const generation = graphRequestGate.next();
  graphAbortController?.abort();
  const controller = new AbortController();
  graphAbortController = controller;
  graphStatus.classList.remove("error");
  graphStatus.textContent = "Loading event relationships…";
  const query = new URLSearchParams();
  if (filters.eventType) query.set("type", filters.eventType);
  if (filters.source) query.set("source", filters.source);
  if (filters.correlation) query.set("correlation_id", filters.correlation);
  query.set("limit", filters.limit);
  try {
    const result = await fetchJson(`/api/event-graph?${query}`, { signal: controller.signal });
    if (!graphRequestGate.isCurrent(generation)) return;
    graph = result;
    graphFilters = filters;
    const parameters = new URLSearchParams(location.search);
    for (const name of ["graph_type", "graph_source", "graph_correlation", "graph_limit"]) parameters.delete(name);
    if (filters.eventType) parameters.set("graph_type", filters.eventType);
    if (filters.source) parameters.set("graph_source", filters.source);
    if (filters.correlation) parameters.set("graph_correlation", filters.correlation);
    parameters.set("graph_limit", filters.limit);
    history.replaceState(null, "", `/trace?${parameters}`);
    if (!graph.events.some(event => event.id === selectedGraphId)) selectedGraphId = null;
    pruneLiveIds(recentLiveIds, result.events);
    const arrivingIds = new Set(intersectEventIds(liveIds, result.events));
    arrivingIds.forEach(id => recentLiveIds.set(id, Date.now() + 6_000));
    scheduleLiveExpiry();
    renderGraph({ preserveViewport, viewportAnchor, arrivingIds });
    subscribeGraph();
  } catch (error) {
    if (!graphRequestGate.isCurrent(generation) || error.name === "AbortError") return;
    if (graph) subscribeGraph();
    throw error;
  } finally {
    if (graphRequestGate.isCurrent(generation)) graphAbortController = null;
  }
}

function renderGraph({ preserveViewport = false, viewportAnchor = null, arrivingIds = new Set() } = {}) {
  graphViewport.replaceChildren();
  graphEventList.replaceChildren();
  graphEmpty.hidden = graph.events.length > 0;
  if (!graph.events.length) {
    graphPositions.clear();
    graphWorld = { width: 1200, height: 620 };
    selectedGraphId = null;
    hoveredGraphId = null;
    graphFocusButton.disabled = true;
    graphTooltip.hidden = true;
    clearLiveState();
    graphInitialFit = true;
    graphStatus.textContent = "0 events.";
    renderEmptyInspector("No event is available to inspect.");
    return;
  }
  expireLiveIds(recentLiveIds, Date.now());
  const layout = layoutEvents(graph.events);
  const events = layout.ordered;
  const eventById = new Map(events.map(event => [event.id, event]));
  if (hoveredGraphId && !eventById.has(hoveredGraphId)) {
    hoveredGraphId = null;
    graphTooltip.hidden = true;
  }
  graphWorld = { width: layout.width, height: layout.height };
  const positions = layout.positions;
  graphPositions = new Map(positions);
  if (preserveViewport && viewportAnchor && graphPositions.has(viewportAnchor.id)) {
    graphTransform = compensateAnchor(
      graphTransform,
      viewportAnchor.position,
      graphPositions.get(viewportAnchor.id),
    );
  }

  const causalEdges = [];
  const boundaryParents = new Map();
  events.forEach(event => {
    if (!event.causation_id) return;
    if (eventById.has(event.causation_id)) causalEdges.push({ source: event.causation_id, target: event.id, kind: "causal" });
    else {
      const child = positions.get(event.id);
      if (!boundaryParents.has(event.causation_id)) {
        boundaryParents.set(event.causation_id, { x: Math.max(20, child.x - 42), y: Math.max(24, child.y - 25) });
      }
      causalEdges.push({ source: event.causation_id, target: event.id, kind: "causal", boundary: true });
    }
  });
  boundaryParents.forEach((position, id) => positions.set(id, position));

  let boundaryLinks = 0;
  const semanticEdges = graph.links.flatMap(link => {
    if (!eventById.has(link.from_event_id) || !eventById.has(link.to_event_id)) {
      boundaryLinks += 1;
      return [];
    }
    return [{ source: link.from_event_id, target: link.to_event_id, kind: "semantic", link }];
  });
  [...causalEdges, ...semanticEdges].forEach(edge => graphViewport.append(graphEdge(edge, positions, eventById)));
  boundaryParents.forEach((position, id) => graphViewport.append(boundaryNode(id, position)));
  events.forEach(event => {
    graphViewport.append(graphNode(event, positions.get(event.id), events.length <= 120, arrivingIds.has(event.id)));
    graphEventList.append(accessibleGraphNode(event));
  });
  applyGraphHighlight();
  const selected = events.find(event => event.id === selectedGraphId);
  graphFocusButton.disabled = !selected;
  if (selected) renderGraphInspector(selected);
  else renderEmptyInspector("Select an event to highlight its correlation and inspect its complete data.");
  if (!preserveViewport || graphInitialFit) requestAnimationFrame(fitGraph);
  else applyGraphTransform();
  graphInitialFit = false;
  const notices = [];
  if (graph.events_truncated) notices.push("older events omitted by the node limit");
  if (graph.links_truncated) notices.push("semantic links capped at 2,000");
  if (boundaryParents.size) notices.push(`${boundaryParents.size} causal parents outside the window`);
  if (boundaryLinks) notices.push(`${boundaryLinks} semantic links cross the visible boundary`);
  graphStatus.textContent = `${events.length} events · ${causalEdges.length} causal edges · ${semanticEdges.length} visible semantic edges${notices.length ? ` · ${notices.join(" · ")}` : ""}.`;
}

function graphEdge(edge, positions, eventById) {
  const source = positions.get(edge.source);
  const target = positions.get(edge.target);
  const bidirectional = Boolean(edge.link?.bidirectional);
  const trimmed = trimLine(source, target, bidirectional ? 13 : edge.boundary ? 10 : 9, 13);
  const pathData = trimmed
    ? `M ${trimmed.source.x} ${trimmed.source.y} L ${trimmed.target.x} ${trimmed.target.y}`
    : `M ${source.x + 10} ${source.y} C ${source.x + 28} ${source.y - 28}, ${source.x - 28} ${source.y - 28}, ${source.x - 10} ${source.y}`;
  const path = svgEl("path", {
    d: pathData,
    class: `graph-edge ${edge.kind}${bidirectional ? " bidirectional" : ""}`,
    "data-source": edge.source,
    "data-target": edge.target,
  });
  const sourceCorrelation = eventById.get(edge.source)?.correlation_id;
  const targetCorrelation = eventById.get(edge.target)?.correlation_id;
  if (sourceCorrelation) path.dataset.sourceCorrelation = sourceCorrelation;
  if (targetCorrelation) path.dataset.targetCorrelation = targetCorrelation;
  const title = svgEl("title");
  title.textContent = edge.kind === "causal"
    ? `Causation: ${short(edge.source)} → ${short(edge.target)}`
    : `${edge.link.relation}${edge.link.description ? ` — ${edge.link.description}` : ""}`;
  path.append(title);
  return path;
}

function graphNode(event, position, showLabel, arriving) {
  const live = recentLiveIds.has(event.id);
  const group = svgEl("g", {
    class: `event-graph-node${live ? " live" : ""}${arriving ? " arriving" : ""}`,
    transform: `translate(${position.x} ${position.y})`,
    "data-id": event.id,
    "data-correlation": event.correlation_id,
  });
  const circle = svgEl("circle", { class: "graph-node-circle", r: selectedGraphId === event.id ? 9 : 7 });
  const title = svgEl("title");
  title.textContent = `${event.event_type}\n${event.id}\nCorrelation ${event.correlation_id}`;
  circle.append(title);
  group.append(circle);
  if (showLabel) {
    const label = svgEl("text", { x: 11, y: 4 });
    label.textContent = event.event_type.length > 28 ? `${event.event_type.slice(0, 27)}…` : event.event_type;
    group.append(label);
  }
  return group;
}

function accessibleGraphNode(event) {
  const item = el("div");
  item.setAttribute("role", "listitem");
  const live = recentLiveIds.has(event.id);
  const button = el("button", live ? "live" : "", `${live ? "New · " : ""}E${event.sequence} · ${event.event_type} · correlation ${short(event.correlation_id)}`);
  button.type = "button";
  button.dataset.id = event.id;
  button.dataset.label = `E${event.sequence} · ${event.event_type} · correlation ${short(event.correlation_id)}`;
  button.setAttribute("aria-pressed", String(event.id === selectedGraphId));
  button.title = `${event.event_type}\n${event.id}\nCorrelation ${event.correlation_id}`;
  button.addEventListener("click", () => {
    selectGraphEvent(event.id);
    centerGraphEvent(event.id);
  });
  item.append(button);
  return item;
}

function boundaryNode(id, position) {
  const group = svgEl("g", {
    class: "event-graph-node boundary",
    transform: `translate(${position.x} ${position.y})`,
    "aria-label": `Parent ${id} outside visible window`,
  });
  group.append(svgEl("rect", { x: -6, y: -6, width: 12, height: 12, rx: 2 }));
  const title = svgEl("title");
  title.textContent = `Causal parent outside visible window: ${id}`;
  group.append(title);
  return group;
}

function selectGraphEvent(id) {
  selectedGraphId = id;
  graphFocusButton.disabled = false;
  applyGraphHighlight();
  const event = graph.events.find(candidate => candidate.id === id);
  if (event) renderGraphInspector(event);
}

function applyGraphHighlight() {
  const selected = graph?.events.find(event => event.id === selectedGraphId);
  const correlation = selected?.correlation_id;
  const neighbors = new Set(hoveredGraphId ? [hoveredGraphId] : []);
  graphViewport.querySelectorAll(".graph-edge").forEach(edge => {
    const hovered = Boolean(hoveredGraphId) && (edge.dataset.source === hoveredGraphId || edge.dataset.target === hoveredGraphId);
    if (hovered) {
      neighbors.add(edge.dataset.source);
      neighbors.add(edge.dataset.target);
    }
    const correlated = Boolean(correlation) && (edge.dataset.sourceCorrelation === correlation || edge.dataset.targetCorrelation === correlation);
    edge.classList.toggle("hovered", hovered);
    edge.classList.toggle("highlighted", !hoveredGraphId && correlated);
    edge.classList.toggle("dimmed", hoveredGraphId ? !hovered : Boolean(correlation) && !correlated);
  });
  graphViewport.querySelectorAll(".event-graph-node:not(.boundary)").forEach(node => {
    const isSelected = node.dataset.id === selectedGraphId;
    const isHovered = node.dataset.id === hoveredGraphId;
    const isNeighbor = Boolean(hoveredGraphId) && neighbors.has(node.dataset.id) && !isHovered;
    node.classList.toggle("selected", isSelected);
    node.classList.toggle("hovered", isHovered);
    node.classList.toggle("neighbor", isNeighbor);
    node.classList.toggle("correlated", !hoveredGraphId && Boolean(correlation) && node.dataset.correlation === correlation && !isSelected);
    node.classList.toggle("dimmed", hoveredGraphId ? !neighbors.has(node.dataset.id) : Boolean(correlation) && node.dataset.correlation !== correlation);
    node.querySelector(".graph-node-circle")?.setAttribute("r", isSelected || isHovered ? "9" : "7");
  });
  graphEventList.querySelectorAll("button[data-id]").forEach(button => {
    button.setAttribute("aria-pressed", String(button.dataset.id === selectedGraphId));
  });
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
  inspector.replaceChildren();
  const heading = el("div", "trace-inspector-heading");
  heading.append(el("p", "eyebrow", "EVENT GRAPH NODE"), el("h2", "", event.event_type), el("time", "muted", new Date(event.occurred_at).toLocaleString()));
  const grid = el("div", "trace-identity-grid");
  grid.append(identity("ID", event.id), identity("Sequence", String(event.sequence)), identity("Cause", event.causation_id), identity("Correlation", event.correlation_id));
  const relationships = graph.links.filter(link => link.from_event_id === event.id || link.to_event_id === event.id);
  const open = el("button", "primary", "Open causal trace");
  open.type = "button";
  open.addEventListener("click", () => openGraphTrace(event.id));
  inspector.append(heading, grid, open, section("Event payload", event.payload));
  if (relationships.length) inspector.append(section("Semantic relationships", relationships));
  inspector.append(disclosure("Complete record", event));
}

function captureGraphAnchor() {
  if (!graphPositions.size) return null;
  if (selectedGraphId && graphPositions.has(selectedGraphId)) {
    return { id: selectedGraphId, position: { ...graphPositions.get(selectedGraphId) } };
  }
  const width = graphSvg.clientWidth || 800;
  const height = graphSvg.clientHeight || 620;
  const center = { x: width / 2, y: height / 2 };
  let best = null;
  let bestDistance = Infinity;
  for (const [id, position] of graphPositions) {
    const screen = {
      x: graphTransform.x + position.x * graphTransform.scale,
      y: graphTransform.y + position.y * graphTransform.scale,
    };
    if (screen.x < 0 || screen.x > width || screen.y < 0 || screen.y > height) continue;
    const distance = Math.hypot(screen.x - center.x, screen.y - center.y);
    if (distance < bestDistance) {
      best = { id, position: { ...position } };
      bestDistance = distance;
    }
  }
  return best;
}

function openGraphTrace(id) {
  setView("timeline", { load: false });
  openTrace("event_id", id).catch(showError);
}

function graphIdAtClientPoint(clientX, clientY, radius = 20) {
  const bounds = graphSvg.getBoundingClientRect();
  return nearestNode(graphPositions, {
    x: (clientX - bounds.left - graphTransform.x) / graphTransform.scale,
    y: (clientY - bounds.top - graphTransform.y) / graphTransform.scale,
  }, radius / graphTransform.scale);
}

function showGraphHover(clientX, clientY) {
  const id = graphIdAtClientPoint(clientX, clientY, 18);
  if (hoveredGraphId !== id) {
    hoveredGraphId = id;
    applyGraphHighlight();
  }
  if (!id) {
    graphTooltip.hidden = true;
    return;
  }
  const event = graph.events.find(candidate => candidate.id === id);
  if (!event) return;
  graphTooltip.textContent = `${event.event_type}\n${event.source} · E${event.sequence}\nCorrelation ${event.correlation_id}`;
  const stage = graphTooltip.parentElement;
  const bounds = stage.getBoundingClientRect();
  graphTooltip.hidden = false;
  graphTooltip.style.right = "auto";
  graphTooltip.style.left = `${clientX - bounds.left + 12}px`;
  graphTooltip.style.top = `${clientY - bounds.top + 12}px`;
  const maximumLeft = Math.max(8, stage.clientWidth - graphTooltip.offsetWidth - 8);
  const maximumTop = Math.max(8, stage.clientHeight - graphTooltip.offsetHeight - 8);
  graphTooltip.style.left = `${Math.min(maximumLeft, Math.max(8, clientX - bounds.left + 12))}px`;
  graphTooltip.style.top = `${Math.min(maximumTop, Math.max(8, clientY - bounds.top + 12))}px`;
}

function clearGraphHover() {
  if (!hoveredGraphId && graphTooltip.hidden) return;
  hoveredGraphId = null;
  graphTooltip.hidden = true;
  applyGraphHighlight();
}

function focusGraphEvent(id) {
  graphTransform.scale = Math.max(graphTransform.scale, 0.8);
  centerGraphEvent(id);
}

function centerGraphEvent(id) {
  const position = graphPositions.get(id);
  if (!position) return;
  graphTransform.x = graphSvg.clientWidth / 2 - position.x * graphTransform.scale;
  graphTransform.y = graphSvg.clientHeight / 2 - position.y * graphTransform.scale;
  applyGraphTransform();
}

function applyGraphTransform() {
  graphViewport.setAttribute("transform", `translate(${graphTransform.x} ${graphTransform.y}) scale(${graphTransform.scale})`);
}

function fitGraph() {
  graphTransform = fitTransform(graphWorld, {
    width: graphSvg.clientWidth || 800,
    height: graphSvg.clientHeight || 620,
  });
  applyGraphTransform();
}

function zoomGraph(factor, centerX = graphSvg.clientWidth / 2, centerY = graphSvg.clientHeight / 2) {
  const next = Math.min(5, Math.max(.12, graphTransform.scale * factor));
  const worldX = (centerX - graphTransform.x) / graphTransform.scale;
  const worldY = (centerY - graphTransform.y) / graphTransform.scale;
  graphTransform.x = centerX - worldX * next;
  graphTransform.y = centerY - worldY * next;
  graphTransform.scale = next;
  applyGraphTransform();
}

function showError(error) {
  status.textContent = error.message;
  status.classList.add("error");
}

function handleGraphError(error) {
  if (error.name === "AbortError") return;
  graphStatus.textContent = error.message;
  graphStatus.classList.add("error");
  graphEmpty.hidden = false;
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
timelineTab.addEventListener("click", () => setView("timeline"));
graphTab.addEventListener("click", () => setView("graph"));
[timelineTab, graphTab].forEach((tab, index, tabs) => {
  tab.addEventListener("keydown", event => {
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
  });
});
graphForm.addEventListener("submit", event => {
  event.preventDefault();
  graphInitialFit = true;
  openGraph().catch(handleGraphError);
});
document.querySelector("#graph-zoom-in").addEventListener("click", () => zoomGraph(1.25));
document.querySelector("#graph-zoom-out").addEventListener("click", () => zoomGraph(.8));
document.querySelector("#graph-fit").addEventListener("click", fitGraph);
graphFocusButton.addEventListener("click", () => {
  if (selectedGraphId) focusGraphEvent(selectedGraphId);
});
graphLiveButton.addEventListener("click", () => {
  graphLiveEnabled = !graphLiveEnabled;
  graphLiveButton.textContent = graphLiveEnabled ? "Pause live" : "Resume live";
  graphLiveButton.setAttribute("aria-pressed", String(graphLiveEnabled));
  if (graphLiveEnabled) subscribeGraph();
  else {
    closeGraphStream();
    setGraphLiveStatus("Live paused", "paused");
  }
});
document.querySelector("#graph-clear").addEventListener("click", () => {
  selectedGraphId = null;
  graphFocusButton.disabled = true;
  applyGraphHighlight();
  renderEmptyInspector("Select an event to highlight its correlation and inspect its complete data.");
});
graphSvg.addEventListener("wheel", event => {
  event.preventDefault();
  const bounds = graphSvg.getBoundingClientRect();
  zoomGraph(event.deltaY < 0 ? 1.12 : .89, event.clientX - bounds.left, event.clientY - bounds.top);
}, { passive: false });
graphSvg.addEventListener("pointerdown", event => {
  if (event.button !== 0) return;
  clearGraphHover();
  graphDrag = { x: event.clientX, y: event.clientY, originX: graphTransform.x, originY: graphTransform.y };
  graphSvg.setPointerCapture(event.pointerId);
  graphSvg.classList.add("dragging");
});
graphSvg.addEventListener("pointermove", event => {
  if (!graphDrag) {
    showGraphHover(event.clientX, event.clientY);
    return;
  }
  graphTransform.x = graphDrag.originX + event.clientX - graphDrag.x;
  graphTransform.y = graphDrag.originY + event.clientY - graphDrag.y;
  applyGraphTransform();
});
graphSvg.addEventListener("pointerup", event => {
  const drag = graphDrag;
  graphDrag = null;
  graphSvg.releasePointerCapture(event.pointerId);
  graphSvg.classList.remove("dragging");
  if (!drag || Math.hypot(event.clientX - drag.x, event.clientY - drag.y) > 4) return;
  const id = graphIdAtClientPoint(event.clientX, event.clientY, 24);
  if (id) selectGraphEvent(id);
});
graphSvg.addEventListener("pointercancel", () => {
  graphDrag = null;
  graphSvg.classList.remove("dragging");
});
graphSvg.addEventListener("pointerleave", () => { if (!graphDrag) clearGraphHover(); });
graphSvg.addEventListener("dblclick", event => {
  const id = graphIdAtClientPoint(event.clientX, event.clientY, 24);
  if (id) openGraphTrace(id);
});

const scheduleGraphRefit = () => {
  clearTimeout(graphResizeTimer);
  graphResizeTimer = setTimeout(() => {
    if (graph && !graphPanel.hidden) fitGraph();
  }, 120);
};
if ("ResizeObserver" in globalThis) new ResizeObserver(scheduleGraphRefit).observe(graphSvg);
else window.addEventListener("resize", scheduleGraphRefit);

const parameters = new URLSearchParams(location.search);
graphType.value = parameters.get("graph_type") || "";
graphSource.value = parameters.get("graph_source") || "";
graphCorrelation.value = parameters.get("graph_correlation") || "";
if (["100", "250", "500", "1000"].includes(parameters.get("graph_limit"))) graphLimit.value = parameters.get("graph_limit");
const eventId = parameters.get("event_id");
const correlationId = parameters.get("correlation_id");
if (eventId) openTrace("event_id", eventId).catch(showError);
else if (correlationId) openTrace("correlation_id", correlationId).catch(showError);
else openLatest().catch(showError);
if (parameters.get("view") === "graph") setView("graph");
window.addEventListener("beforeunload", () => {
  closeGraphStream();
  clearLiveState();
});
