const draftList = document.querySelector("#draft-list");
const fileList = document.querySelector("#file-list");
const editor = document.querySelector("#file-editor");
const fileLabel = document.querySelector("#file-label");
const saveFile = document.querySelector("#save-file");
const validateDraft = document.querySelector("#validate-draft");
const installDraft = document.querySelector("#install-draft");
const validationPanel = document.querySelector("#validation");
const status = document.querySelector("#studio-status");
const newFile = document.querySelector("#new-file");
const newDirectory = document.querySelector("#new-directory");

let selectedDraft = null;
let selectedFile = null;
let selectedHash = null;
let approvedValidation = null;

async function api(path, options) {
  const response = await fetch(path, options);
  const body = await response.json();
  if (!response.ok) throw new Error(body.error || `${response.status} ${response.statusText}`);
  return body;
}

function setStatus(message) {
  status.textContent = message;
}

function invalidateValidation() {
  approvedValidation = null;
  installDraft.disabled = true;
  validationPanel.textContent = "Draft changed. Validate again before installation.";
  validationPanel.className = "studio-review muted";
}

async function loadDrafts() {
  const drafts = await api("/api/studio/drafts");
  draftList.replaceChildren();
  for (const draft of drafts) {
    const button = document.createElement("button");
    button.className = `studio-list-item${draft.id === selectedDraft ? " selected" : ""}`;
    button.textContent = draft.id;
    button.onclick = () => selectDraft(draft.id);
    draftList.append(button);
  }
  if (!drafts.length) draftList.textContent = "No drafts yet.";
}

async function selectDraft(id) {
  selectedDraft = id;
  selectedFile = null;
  selectedHash = null;
  editor.value = "";
  editor.disabled = true;
  saveFile.disabled = true;
  validateDraft.disabled = false;
  newFile.disabled = false;
  newDirectory.disabled = false;
  invalidateValidation();
  await Promise.all([loadDrafts(), loadFiles()]);
}

async function loadFiles() {
  const result = await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/files`);
  fileList.replaceChildren();
  for (const path of result.files) {
    const button = document.createElement("button");
    button.className = `studio-list-item${path === selectedFile ? " selected" : ""}`;
    button.textContent = path;
    button.onclick = () => openFile(path);
    fileList.append(button);
  }
}

async function openFile(path) {
  const file = await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/files/${path.split("/").map(encodeURIComponent).join("/")}`);
  selectedFile = file.path;
  selectedHash = file.sha256;
  fileLabel.textContent = `${selectedDraft}/${file.path} · ${file.sha256}`;
  editor.value = file.content;
  editor.disabled = false;
  saveFile.disabled = false;
  await loadFiles();
}

document.querySelector("#create-draft").addEventListener("submit", async (event) => {
  event.preventDefault();
  const form = new FormData(event.currentTarget);
  try {
    const draft = await api("/api/studio/drafts", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(Object.fromEntries(form)),
    });
    event.currentTarget.reset();
    await selectDraft(draft.id);
    setStatus(`Created ${draft.id}.`);
  } catch (error) {
    setStatus(error.message);
  }
});

saveFile.onclick = async () => {
  saveFile.disabled = true;
  try {
    const file = await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/files/${selectedFile.split("/").map(encodeURIComponent).join("/")}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content: editor.value, expected_sha256: selectedHash }),
    });
    selectedHash = file.sha256;
    fileLabel.textContent = `${selectedDraft}/${file.path} · ${file.sha256}`;
    invalidateValidation();
    setStatus(`Saved ${file.path}.`);
  } catch (error) {
    setStatus(error.message);
  } finally {
    saveFile.disabled = false;
  }
};

newDirectory.onclick = async () => {
  const path = window.prompt("Relative directory path");
  if (!path) return;
  try {
    await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/directories`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path }),
    });
    invalidateValidation();
    setStatus(`Created ${path}.`);
  } catch (error) {
    setStatus(error.message);
  }
};

newFile.onclick = async () => {
  const path = window.prompt("Relative file path (.toml, .lua, .html, .css, .js, .md, or .json)");
  if (!path) return;
  try {
    await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/files/${path.split("/").map(encodeURIComponent).join("/")}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content: "", expected_sha256: null }),
    });
    invalidateValidation();
    await loadFiles();
    await openFile(path);
  } catch (error) {
    setStatus(error.message);
  }
};

validateDraft.onclick = async () => {
  validateDraft.disabled = true;
  validationPanel.textContent = "Validating…";
  try {
    const result = await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/validate`, { method: "POST" });
    approvedValidation = result.valid ? result : null;
    validationPanel.replaceChildren();
    const summary = document.createElement("pre");
    summary.textContent = [
      `${result.name} ${result.version} (${result.id})`,
      result.content_hash,
      `Capabilities: ${Object.entries(result.capabilities).filter(([, enabled]) => enabled).map(([name]) => name).join(", ") || "none"}`,
      `Scan: ${result.security_scan.passed ? "passed" : "blocked"} · ${result.security_scan.warning_count} warnings · ${result.security_scan.blocker_count} blockers`,
      result.validation_error || "Runtime validation passed",
    ].join("\n");
    validationPanel.append(summary);
    for (const finding of result.security_scan.findings) {
      const item = document.createElement("p");
      item.className = `scan-status ${finding.severity === "blocker" ? "failed" : ""}`;
      item.textContent = `${finding.severity}: ${finding.file} — ${finding.message}`;
      validationPanel.append(item);
    }
    validationPanel.className = "studio-review";
    installDraft.disabled = !result.valid;
  } catch (error) {
    approvedValidation = null;
    validationPanel.textContent = error.message;
    installDraft.disabled = true;
  } finally {
    validateDraft.disabled = false;
  }
};

installDraft.onclick = async () => {
  if (!approvedValidation) return;
  const capabilities = Object.entries(approvedValidation.capabilities).filter(([, enabled]) => enabled).map(([name]) => name).join(", ") || "none";
  if (!window.confirm(`Install ${approvedValidation.id} ${approvedValidation.version}?\n\nHash: ${approvedValidation.content_hash}\nCapabilities: ${capabilities}\n\nInstalled extensions are fully trusted local code.`)) return;
  installDraft.disabled = true;
  try {
    const result = await api(`/api/studio/drafts/${encodeURIComponent(selectedDraft)}/install`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ approved_hash: approvedValidation.content_hash }),
    });
    setStatus(`Installed ${result.installed.name} ${result.installed.version}.`);
  } catch (error) {
    setStatus(error.message);
  }
};

loadDrafts().catch((error) => setStatus(error.message));
