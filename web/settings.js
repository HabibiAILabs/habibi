const fields = ["directory_includes", "directory_excludes", "program_includes", "program_excludes"];
const form = document.querySelector("#boundary-form");
const status = document.querySelector("#boundary-status");

function element(key) { return document.querySelector(`#${key.replaceAll("_", "-")}`); }
function lines(value) { return value.split("\n").map(item => item.trim()).filter(Boolean); }
function show(policy) { for (const key of fields) element(key).value = (policy[key] || []).join("\n"); }

try {
  const response = await fetch("/api/settings/boundaries");
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `Could not load boundaries (${response.status})`);
  show(body);
} catch (error) { status.textContent = error.message; }

form.addEventListener("submit", async event => {
  event.preventDefault();
  const button = form.querySelector("button");
  button.disabled = true;
  status.textContent = "Saving…";
  try {
    const policy = Object.fromEntries(fields.map(key => [key, lines(element(key).value)]));
    const response = await fetch("/api/settings/boundaries", { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify(policy) });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error || `Could not save boundaries (${response.status})`);
    show(body);
    status.textContent = "Saved.";
  } catch (error) { status.textContent = error.message; }
  finally { button.disabled = false; }
});
