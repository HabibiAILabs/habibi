const number = new Intl.NumberFormat();
const money = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 6 });
const $ = (selector) => document.querySelector(selector);

function cost(value) { return value == null ? "Unavailable" : money.format(value); }
function cacheRate(usage) {
  const prompt = usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
  return prompt ? usage.cache_read_tokens / prompt : 0;
}
async function jsonRequest(path, options) {
  const response = await fetch(path, options), result = await response.json();
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
  return result;
}

async function loadStats() {
  const usage = (await jsonRequest("/api/stats")).usage;
  $("#total-cost").textContent = cost(usage.estimated_cost_usd);
  $("#cost-coverage").textContent = usage.priced_invocations === usage.invocations
    ? `${number.format(usage.priced_invocations)} priced invocations`
    : `${number.format(usage.priced_invocations)} of ${number.format(usage.invocations)} priced`;
  $("#total-tokens").textContent = number.format(usage.total_tokens);
  $("#invocations").textContent = `${number.format(usage.invocations)} completed invocations`;
  $("#cache-tokens").textContent = number.format(usage.cache_read_tokens);
  $("#cache-rate").textContent = `${(cacheRate(usage) * 100).toFixed(1)}% prompt cache hit rate`;
  $("#output-tokens").textContent = number.format(usage.output_tokens);
  $("#failed-invocations").textContent = `${number.format(usage.failed_invocations)} failed invocations`;
  $("#model-usage").replaceChildren(...usage.models.map((model) => row([
    `${model.provider} / ${model.model}`, number.format(model.invocations), number.format(model.input_tokens),
    number.format(model.cache_read_tokens), number.format(model.cache_write_tokens),
    number.format(model.output_tokens), number.format(model.total_tokens), cost(model.estimated_cost_usd),
  ])));
  if (!usage.models.length) emptyRow("#model-usage", 8, "No completed model invocations yet.");
  $("#event-type-usage").replaceChildren(...usage.event_types.map((eventType) => row([
    link(eventType.event_type, `/events?type=${encodeURIComponent(eventType.event_type)}`),
    number.format(eventType.invocations),
    number.format(eventType.input_tokens),
    number.format(eventType.output_tokens),
    number.format(eventType.total_tokens),
    eventType.average_duration_ms == null ? "—" : `${eventType.average_duration_ms.toFixed(1)} ms`,
    eventType.last_invocation_at ? new Date(eventType.last_invocation_at).toLocaleString() : "—",
    link("Logs", `/logs?name=model.invocation.started&payload_contains=${encodeURIComponent(eventType.event_type)}`),
  ])));
  if (!usage.event_types.length) emptyRow("#event-type-usage", 8, "No event-triggered model invocations yet.");
  $("#tool-usage").replaceChildren(...usage.tools.map((tool) => row([
    tool.tool,
    number.format(tool.advertised_invocations),
    number.format(tool.dispatches_advertised),
    number.format(tool.calls),
    number.format(tool.dispatches_used),
    tool.advertised_invocations ? `${((tool.calls / tool.advertised_invocations) * 100).toFixed(1)}%` : "—",
    number.format(tool.succeeded),
    number.format(tool.failed),
    number.format(tool.estimated_schema_tokens),
    tool.average_duration_ms == null ? "—" : `${tool.average_duration_ms.toFixed(1)} ms`,
  ])));
  if (!usage.tools.length) emptyRow("#tool-usage", 10, "No tool advertisements or calls yet.");
}

async function loadCatalog() {
  const result = await jsonRequest("/api/models");
  const catalog = result.catalog;
  $("#active-model").textContent = `${result.active.provider} / ${result.active.model} · change HABIBI_MODEL_PROVIDER and HABIBI_MODEL, then restart Habibi to switch.`;
  $("#catalog-status").textContent = `${number.format(catalog.models.length)} priced models · catalog updated ${new Date(catalog.updated_at).toLocaleString()} · source: ${catalog.source}`;
  $("#model-catalog").replaceChildren(...catalog.models.map((model) => row([
    `${model.provider} / ${model.id}`,
    cost(model.pricing.input_usd_per_million),
    cost(model.pricing.cache_read_usd_per_million),
    cost(model.pricing.cache_write_usd_per_million),
    cost(model.pricing.output_usd_per_million),
    model.updated_at ? new Date(model.updated_at).toLocaleDateString() : "—",
  ])));
  if (!catalog.models.length) emptyRow("#model-catalog", 6, "No priced models in the catalog.");
}

function row(values) {
  const row = document.createElement("tr");
  for (const value of values) {
    const cell = document.createElement("td");
    if (value instanceof Node) cell.append(value); else cell.textContent = value;
    row.append(cell);
  }
  return row;
}
function link(label, href) {
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.textContent = label;
  return anchor;
}
function emptyRow(selector, span, message) {
  const tableRow = document.createElement("tr"), cell = document.createElement("td");
  cell.colSpan = span; cell.className = "muted"; cell.textContent = message; tableRow.append(cell); $(selector).append(tableRow);
}

$("#refresh-prices").addEventListener("click", async () => {
  const button = $("#refresh-prices");
  button.disabled = true; button.textContent = "Refreshing…";
  try {
    const catalog = (await jsonRequest("/api/models/refresh", { method: "POST" })).catalog;
    $("#catalog-status").textContent = `Refreshed ${number.format(catalog.models.length)} model prices.`;
    await Promise.all([loadCatalog(), loadStats()]);
  } catch (error) {
    $("#catalog-status").textContent = `Refresh failed: ${error.message}`;
  } finally {
    button.disabled = false; button.textContent = "Refresh model prices";
  }
});

Promise.all([loadStats(), loadCatalog()]).catch((error) => { $("#pricing-note").textContent = error.message; });
