const id = decodeURIComponent(location.pathname.split("/").filter(Boolean)[2] || "");
const title = document.querySelector("#kv-title");
const status = document.querySelector("#kv-status");
const list = document.querySelector("#kv-list");
const filter = document.querySelector("#kv-filter");
let entries = [];

function render() {
  const query = filter.value.toLowerCase();
  const visible = entries.filter(entry => entry.key.toLowerCase().includes(query));
  list.replaceChildren(...visible.map(entry => {
    const card = document.createElement("article");
    card.className = "kv-entry";
    const heading = document.createElement("header");
    const key = document.createElement("strong"); key.textContent = entry.key;
    const time = document.createElement("time"); time.dateTime = entry.updated_at; time.textContent = new Date(entry.updated_at).toLocaleString();
    heading.append(key, time);
    const value = document.createElement("pre"); value.textContent = JSON.stringify(entry.value, null, 2);
    card.append(heading, value); return card;
  }));
  status.textContent = `${visible.length} of ${entries.length} keys`;
}

filter.addEventListener("input", render);
try {
  const extensionsResponse = await fetch("/api/extensions");
  const extensions = await extensionsResponse.json();
  const extension = extensions.find(item => item.id === id);
  if (extension) { title.textContent = `${extension.name} KV`; document.title = `${extension.name} KV · Habibi`; }
  const response = await fetch(`/api/extensions/${encodeURIComponent(id)}/kv`);
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `Could not load KV (${response.status})`);
  entries = body.entries; render();
} catch (error) { status.textContent = error.message; }
