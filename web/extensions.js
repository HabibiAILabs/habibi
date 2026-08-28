const list = document.querySelector("#extension-list");

async function loadExtensions() {
  const response = await fetch("/api/extensions");
  if (!response.ok) throw new Error(`Could not load extensions (${response.status})`);
  const extensions = await response.json();
  list.replaceChildren(...extensions.map(extensionCard));
}

function extensionCard(extension) {
  const card = document.createElement("article");
  card.className = `extension-card${extension.enabled ? "" : " disabled"}`;

  const details = document.createElement("div");
  const title = document.createElement("div");
  title.className = "extension-title";
  const heading = document.createElement("h2");
  heading.textContent = extension.name;
  const version = document.createElement("span");
  version.className = "version";
  version.textContent = `${extension.id} · v${extension.version}`;
  title.append(heading, version);

  const description = document.createElement("p");
  description.className = "description";
  description.textContent = extension.description || "No description provided.";
  const provides = document.createElement("ul");
  provides.className = "provides";
  for (const capability of extension.provides) {
    const item = document.createElement("li");
    item.textContent = capability;
    provides.append(item);
  }
  details.append(title, description, provides);

  const actions = document.createElement("div");
  actions.className = "extension-actions";
  if (extension.main_page) {
    const open = document.createElement("a");
    open.className = "open-link";
    open.href = extension.main_page;
    open.textContent = "Open";
    open.hidden = !extension.enabled;
    actions.append(open);
  }
  const toggle = document.createElement("button");
  toggle.className = `toggle${extension.enabled ? " on" : ""}`;
  toggle.title = extension.enabled ? "Disable extension" : "Enable extension";
  toggle.setAttribute("aria-label", toggle.title);
  toggle.setAttribute("aria-pressed", String(extension.enabled));
  toggle.onclick = async () => {
    toggle.disabled = true;
    try {
      const response = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}`, {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ enabled: !extension.enabled }),
      });
      if (!response.ok) throw new Error(`Update failed (${response.status})`);
      await loadExtensions();
    } finally {
      toggle.disabled = false;
    }
  };
  actions.append(toggle);
  card.append(details, actions);
  return card;
}

loadExtensions().catch((error) => {
  list.innerHTML = `<p class="muted"></p>`;
  list.firstElementChild.textContent = error.message;
});
