const id = decodeURIComponent(location.pathname.split("/").filter(Boolean)[2] || "");
const form = document.querySelector("#config-form");
const value = document.querySelector("#config-value");
const schema = document.querySelector("#config-schema");
const status = document.querySelector("#config-status");
const title = document.querySelector("#config-title");

try {
  const extensions = await (await fetch("/api/extensions")).json();
  const extension = extensions.find(item => item.id === id);
  if (extension) { title.textContent = `${extension.name} Configuration`; document.title = `${extension.name} Configuration · Habibi`; }
  const response = await fetch(`/api/extensions/${encodeURIComponent(id)}/config`); const body = await response.json();
  if (!response.ok) throw new Error(body.error || `Could not load configuration (${response.status})`);
  value.value = JSON.stringify(body.value, null, 2); schema.textContent = JSON.stringify(body.schema, null, 2);
} catch (error) { status.textContent = error.message; }

form.addEventListener("submit", async event => {
  event.preventDefault(); const button = form.querySelector("button"); button.disabled = true; status.textContent = "Saving…";
  try {
    const parsed = JSON.parse(value.value);
    const response = await fetch(`/api/extensions/${encodeURIComponent(id)}/config`, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(parsed) });
    const body = await response.json(); if (!response.ok) throw new Error(body.error || `Could not save configuration (${response.status})`);
    value.value = JSON.stringify(body.value, null, 2); status.textContent = "Saved.";
  } catch (error) { status.textContent = error.message; }
  finally { button.disabled = false; }
});
