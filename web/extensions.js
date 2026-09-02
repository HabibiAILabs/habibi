const list = document.querySelector("#extension-list");

const CAPABILITIES = {
  tools: ["Tools", "Registers functions callable by the model."],
  context: ["Context", "Adds extension-authored text to model system context."],
  web: ["Web", "Registers same-origin routes and extension application pages."],
  kv: ["KV", "Stores JSON values in a private core-managed namespace."],
  events: ["Events", "Queries bounded durable event history."],
  filesystem: ["Filesystem", "Accesses paths allowed by the global directory boundary."],
  process: ["Process", "Runs programs allowed by the global program and directory boundaries."],
  search: ["Search", "Uses the configured host web-search adapter."],
};

function initials(name) { return name.split(/\s+/).map(part => part[0]).join("").slice(0, 2).toUpperCase(); }
function extensionLogo(extension) {
  const visual = document.createElement("span"); visual.className = "extension-logo";
  if (extension.icon) { const image = document.createElement("img"); image.src = extension.icon; image.alt = ""; visual.append(image); }
  else visual.textContent = initials(extension.name);
  return visual;
}
function capabilityList(extension) {
  const section = document.createElement("div"); section.className = "capability-section";
  const label = document.createElement("span"); label.className = "capability-label"; label.textContent = "Capabilities";
  const pills = document.createElement("ul"); pills.className = "capability-pills";
  for (const [key, enabled] of Object.entries(extension.capabilities)) {
    if (!enabled) continue;
    const [name, description] = CAPABILITIES[key] || [key, "Declared extension capability."];
    const item = document.createElement("li"); item.tabIndex = 0; item.textContent = name; item.title = description;
    item.setAttribute("aria-label", `${name}: ${description}`); pills.append(item);
  }
  section.append(label, pills); return section;
}
async function loadExtensions() {
  const response = await fetch("/api/extensions");
  if (!response.ok) throw new Error(`Could not load extensions (${response.status})`);
  list.replaceChildren(...(await response.json()).map(extensionCard));
}
function extensionCard(extension) {
  const card = document.createElement("article"); card.className = `extension-card${extension.enabled ? "" : " disabled"}`;
  const identity = document.createElement("div"); identity.className = "extension-identity";
  const details = document.createElement("div");
  const title = document.createElement("div"); title.className = "extension-title";
  const heading = document.createElement("h2"); heading.textContent = extension.name;
  const version = document.createElement("span"); version.className = "version"; version.textContent = `${extension.id} · v${extension.version}`;
  title.append(heading, version);
  const description = document.createElement("p"); description.className = "description"; description.textContent = extension.description || "No description provided.";
  details.append(title, description, capabilityList(extension));
  if (extension.provides.length) {
    const registrations = document.createElement("p"); registrations.className = "registration-summary";
    registrations.textContent = extension.provides.join(" · "); details.append(registrations);
  }
  if (extension.installation) {
    const source = document.createElement("p"); source.className = "installation-source";
    const installation = extension.installation;
    source.textContent = installation.source.kind === "git" ? `Installed from ${installation.source.url} @ ${installation.source.revision.slice(0, 8)}` : `Installed from ${installation.source.path}`;
    details.append(source);
    const scan = installation.security_scan;
    if (scan) {
      const security = document.createElement("p"); security.className = `scan-status${scan.passed ? " passed" : " failed"}`;
      security.textContent = scan.passed ? `Security/privacy scan passed · ${scan.warning_count} warning${scan.warning_count === 1 ? "" : "s"}` : `Security/privacy scan blocked · ${scan.blocker_count} issue${scan.blocker_count === 1 ? "" : "s"}`;
      if (scan.findings.length) security.title = scan.findings.map(finding => `${finding.file}: ${finding.message}`).join("\n"); details.append(security);
    }
  }
  const actions = document.createElement("div"); actions.className = "extension-actions";
  if (extension.main_page) { const open = document.createElement("a"); open.className = "open-link"; open.href = extension.main_page; open.textContent = "Open"; open.hidden = !extension.enabled; actions.append(open); }
  if (extension.config_page) { const config = document.createElement("a"); config.className = "open-link"; config.href = extension.config_page; config.textContent = "Configure"; actions.append(config); }
  if (extension.capabilities.kv || extension.config_page) { const kv = document.createElement("a"); kv.className = "open-link"; kv.href = `/admin/extensions/${encodeURIComponent(extension.id)}/kv`; kv.textContent = "KV Explorer"; actions.append(kv); }
  if (extension.installation) {
    const update = document.createElement("button"); update.className = "open-link update-button"; update.textContent = "Check update";
    update.onclick = async () => {
      update.disabled = true;
      try {
        const response = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}/check-update`, { method: "POST" }); const result = await response.json();
        if (!response.ok) throw new Error(result.error || `Update check failed (${response.status})`);
        if (!result.update_available) { update.textContent = "Up to date"; return; }
        update.textContent = `Update to ${result.available_version}`; update.disabled = false;
        update.onclick = async () => {
          const capabilities = Object.entries(result.available_capabilities).filter(([, enabled]) => enabled).map(([name]) => name).join(", ") || "none";
          if (!confirm(`Update ${extension.name} from ${result.installed_version} to ${result.available_version}?\n\nCapabilities: ${capabilities}\nRevision: ${(result.available_revision || "local source").slice(0, 12)}`)) return;
          update.disabled = true; update.textContent = "Updating…";
          const apply = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}/update`, { method: "POST" }); const body = await apply.json();
          if (!apply.ok) throw new Error(body.error || `Update failed (${apply.status})`); await loadExtensions();
        };
      } catch (error) { update.textContent = error.message; update.title = error.message; }
      finally { if (update.textContent === "Check update") update.disabled = false; }
    }; actions.append(update);
  }
  const toggle = document.createElement("button"); toggle.className = `toggle${extension.enabled ? " on" : ""}`;
  toggle.title = extension.enabled ? "Disable extension" : "Enable extension"; toggle.setAttribute("aria-label", toggle.title); toggle.setAttribute("aria-pressed", String(extension.enabled));
  toggle.onclick = async () => { toggle.disabled = true; try { const response = await fetch(`/api/extensions/${encodeURIComponent(extension.id)}`, { method: "PUT", headers: { "content-type": "application/json" }, body: JSON.stringify({ enabled: !extension.enabled }) }); if (!response.ok) throw new Error(`Update failed (${response.status})`); await loadExtensions(); } finally { toggle.disabled = false; } };
  actions.append(toggle); identity.append(extensionLogo(extension), details); card.append(identity, actions); return card;
}
loadExtensions().catch(error => { list.innerHTML = `<p class="muted"></p>`; list.firstElementChild.textContent = error.message; });
