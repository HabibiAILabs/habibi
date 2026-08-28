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
  if (extension.installation) {
    const source = document.createElement("p");
    source.className = "installation-source";
    const installation = extension.installation;
    source.textContent = installation.source.kind === "git"
      ? `Installed from ${installation.source.url} @ ${installation.source.revision.slice(0, 8)}`
      : `Installed from ${installation.source.path}`;
    details.append(source);
    const scan = installation.security_scan;
    if (scan) {
      const security = document.createElement("p");
      security.className = `scan-status${scan.passed ? " passed" : " failed"}`;
      security.textContent = scan.passed
        ? `Security/privacy scan passed · ${scan.warning_count} warning${scan.warning_count === 1 ? "" : "s"}`
        : `Security/privacy scan blocked · ${scan.blocker_count} issue${scan.blocker_count === 1 ? "" : "s"}`;
      if (scan.findings.length) security.title = scan.findings.map((finding) => `${finding.file}: ${finding.message}`).join("\n");
      details.append(security);
    }
  }

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
  if (extension.installation) {
    const update = document.createElement("button");
    update.className = "open-link update-button";
    update.textContent = "Check update";
    update.onclick = async () => {
      update.disabled = true;
      try {
        const response = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}/check-update`, { method: "POST" });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || `Update check failed (${response.status})`);
        if (!result.update_available) {
          update.textContent = "Up to date";
          return;
        }
        update.textContent = `Update to ${result.available_version}`;
        update.disabled = false;
        update.onclick = async () => {
          const enabledCapabilities = Object.entries(result.available_capabilities)
            .filter(([, enabled]) => enabled)
            .map(([name]) => name)
            .join(", ") || "none";
          if (!window.confirm(`Update ${extension.name} from ${result.installed_version} to ${result.available_version}?\n\nCapabilities: ${enabledCapabilities}\nRevision: ${(result.available_revision || "local source").slice(0, 12)}`)) return;
          update.disabled = true;
          update.textContent = "Updating…";
          const apply = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}/update`, { method: "POST" });
          const body = await apply.json();
          if (!apply.ok) throw new Error(body.error || `Update failed (${apply.status})`);
          await loadExtensions();
        };
      } catch (error) {
        update.textContent = error.message;
        update.title = error.message;
      } finally {
        if (update.textContent === "Check update") update.disabled = false;
      }
    };
    actions.append(update);
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
