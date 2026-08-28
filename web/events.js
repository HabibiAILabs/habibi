const form = document.querySelector("#event-filters");
const list = document.querySelector("#event-list");
const count = document.querySelector("#event-count");
const range = document.querySelector("#event-range");
const loadOlder = document.querySelector("#load-older");
const customTimes = [...document.querySelectorAll(".custom-time")];
let displayed = [];
let oldestSequence = null;
let lastPageSize = 0;

function queryParameters(beforeSequence) {
  const data = new FormData(form);
  const params = new URLSearchParams();
  for (const [key, rawValue] of data) {
    let value = String(rawValue).trim();
    if (!value || (key === "window" && value === "custom")) continue;
    if ((key === "from" || key === "to") && value) value = new Date(value).toISOString();
    params.set(key, value);
  }
  if (beforeSequence) params.set("before_sequence", beforeSequence);
  return params;
}

async function queryEvents({ older = false } = {}) {
  const response = await fetch(`/api/events?${queryParameters(older ? oldestSequence : null)}`);
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Query failed (${response.status})`);
  const page = result.events.slice().reverse();
  lastPageSize = page.length;
  displayed = older ? [...displayed, ...page] : page;
  oldestSequence = displayed.at(-1)?.sequence || null;
  renderEvents();
}

function renderEvents() {
  count.textContent = `${displayed.length} event${displayed.length === 1 ? "" : "s"}`;
  const newest = displayed[0];
  const oldest = displayed.at(-1);
  range.textContent = newest && oldest ? `#${oldest.sequence} – #${newest.sequence}` : "";
  list.replaceChildren(...displayed.map(eventCard));
  const limit = Number(new FormData(form).get("limit") || 100);
  loadOlder.hidden = lastPageSize < limit || !oldestSequence;
  if (displayed.length === 0) {
    const empty = document.createElement("p");
    empty.className = "muted";
    empty.textContent = "No events match this query.";
    list.append(empty);
  }
}

function eventCard(event) {
  const article = document.createElement("article");
  article.className = `event-card${event.event_type.startsWith("model.invocation.") ? " model-event" : ""}`;

  const header = document.createElement("header");
  const identity = document.createElement("div");
  identity.className = "event-identity";
  const sequence = document.createElement("span");
  sequence.className = "event-sequence";
  sequence.textContent = `#${event.sequence}`;
  const type = document.createElement("strong");
  type.textContent = event.event_type;
  identity.append(sequence, type);
  const timestamp = document.createElement("time");
  timestamp.dateTime = event.occurred_at;
  timestamp.textContent = new Date(event.occurred_at).toLocaleString();
  header.append(identity, timestamp);

  const metadata = document.createElement("div");
  metadata.className = "event-metadata";
  metadata.append(meta("source", event.source), meta("event", event.id), meta("correlation", event.correlation_id));
  if (event.causation_id) metadata.append(meta("caused by", event.causation_id));

  const details = document.createElement("details");
  if (event.event_type === "model.invocation.started") details.className = "reaction-detail";
  const summary = document.createElement("summary");
  summary.textContent = detailLabel(event);
  const payload = document.createElement("pre");
  payload.textContent = JSON.stringify(event.payload, null, 2);
  details.append(summary, payload);
  article.append(header, metadata, details);
  return article;
}

function detailLabel(event) {
  if (event.event_type === "model.invocation.started") return "Exact LLM request (instructions and input)";
  if (event.event_type === "model.invocation.completed") return "Model response and usage";
  if (event.event_type === "model.invocation.failed") return "Model failure";
  if (event.event_type === "action.proposed") return "Tool call and arguments";
  if (event.event_type === "action.succeeded") return "Tool result and effect events";
  if (event.event_type === "action.failed") return "Tool failure";
  if (event.event_type === "action.batch.completed") return "Coalesced batch result references";
  if (event.event_type === "event.link.created") return "Semantic event link";
  return "Event payload";
}

function meta(label, value) {
  const element = document.createElement("span");
  element.title = value;
  element.textContent = `${label}: ${value}`;
  return element;
}

form.elements.window.addEventListener("change", () => {
  const custom = form.elements.window.value === "custom";
  customTimes.forEach((element) => { element.hidden = !custom; });
});
form.addEventListener("submit", (event) => {
  event.preventDefault();
  queryEvents().catch(showError);
});
loadOlder.addEventListener("click", () => queryEvents({ older: true }).catch(showError));
document.querySelector("#model-reactions").addEventListener("click", () => {
  form.reset();
  form.elements.prefix.value = "model.invocation.";
  customTimes.forEach((element) => { element.hidden = true; });
  queryEvents().catch(showError);
});
document.querySelector("#actions").addEventListener("click", () => {
  form.reset();
  form.elements.prefix.value = "action.";
  customTimes.forEach((element) => { element.hidden = true; });
  queryEvents().catch(showError);
});

function showError(error) {
  list.innerHTML = "";
  const message = document.createElement("p");
  message.className = "muted";
  message.textContent = error.message;
  list.append(message);
}

queryEvents().catch(showError);
